use crate::intake::{self, gateway, IntakePayload};
use crate::models::intake::{
    BulkResult, IntakeConfig, ScanResult, SyncResult, UpdateIntakeConfig, WebhookStatus,
};
use crate::models::issue::{CreateIssue, Issue};
use crate::state::AppState;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use tracing::info;

// ── Config ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_intake_config(state: State<'_, AppState>) -> Result<IntakeConfig, String> {
    sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_intake_config(
    payload: UpdateIntakeConfig,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<IntakeConfig, String> {
    // 克隆跨 await 所需的字段
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();
    let webhook_handle = state.webhook_handle.clone();

    // COALESCE pattern：只有传入值不为 NULL 时才覆盖现有值
    sqlx::query(
        "UPDATE intake_configs SET
         webhook_enabled  = COALESCE(?, webhook_enabled),
         webhook_port     = COALESCE(?, webhook_port),
         webhook_token    = COALESCE(?, webhook_token),
         github_owner     = COALESCE(?, github_owner),
         github_repo      = COALESCE(?, github_repo),
         github_token     = COALESCE(?, github_token),
         github_project_id= COALESCE(?, github_project_id),
         ci_watch_dir     = COALESCE(?, ci_watch_dir),
         updated_at       = datetime('now')
         WHERE id='singleton'",
    )
    .bind(payload.webhook_enabled.map(|b| b as i64))
    .bind(payload.webhook_port)
    .bind(payload.webhook_token.as_deref())
    .bind(payload.github_owner.as_deref())
    .bind(payload.github_repo.as_deref())
    .bind(payload.github_token.as_deref())
    .bind(payload.github_project_id.as_deref())
    .bind(payload.ci_watch_dir.as_deref())
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;

    // 若 webhook 相关配置有变，重启 server
    let webhook_changed = payload.webhook_enabled.is_some()
        || payload.webhook_port.is_some()
        || payload.webhook_token.is_some();
    if webhook_changed {
        restart_webhook(&db, &webhook_handle, job_tx, app).await;
    }

    sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_one(&db)
        .await
        .map_err(|e| e.to_string())
}

async fn restart_webhook(
    db: &crate::db::Db,
    handle_arc: &Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    job_tx: crate::tasks::runner::JobSender,
    app: AppHandle,
) {
    let cfg = match sqlx::query_as::<_, IntakeConfig>(
        "SELECT * FROM intake_configs WHERE id='singleton'",
    )
    .fetch_one(db)
    .await
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut guard = handle_arc.lock().await;
    if let Some(old) = guard.take() {
        old.abort();
        info!("[webhook] 已停止旧 server");
    }

    if cfg.webhook_enabled && !cfg.webhook_token.is_empty() {
        let port = cfg.webhook_port as u16;
        let token = cfg.webhook_token.clone();
        let db_clone = db.clone();
        let app_clone = app.clone();
        let new_handle = tokio::spawn(async move {
            if let Err(e) =
                intake::webhook::start(port, token, db_clone, job_tx, app_clone).await
            {
                tracing::error!("[webhook] server error: {}", e);
            }
        });
        *guard = Some(new_handle);
        info!("[webhook] 已在端口 {} 启动", port);
    }
}

// ── Webhook status ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_webhook_status(state: State<'_, AppState>) -> Result<WebhookStatus, String> {
    let db = state.db.clone();
    let webhook_handle = state.webhook_handle.clone();

    let cfg = sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_one(&db)
        .await
        .map_err(|e| e.to_string())?;

    let running = {
        let guard = webhook_handle.lock().await;
        guard.as_ref().map(|h| !h.is_finished()).unwrap_or(false)
    };

    Ok(WebhookStatus {
        running,
        port: cfg.webhook_port as u16,
    })
}

// ── GitHub sync ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn sync_github_issues(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncResult, String> {
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();

    let cfg = sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_one(&db)
        .await
        .map_err(|e| e.to_string())?;

    if cfg.github_owner.is_empty() || cfg.github_repo.is_empty() {
        return Err("请先配置 GitHub Owner 和 Repo".to_string());
    }
    if cfg.github_project_id.is_empty() {
        return Err("请先绑定目标项目".to_string());
    }

    let token_str = cfg.github_token.clone();
    let token_opt = if token_str.is_empty() { None } else { Some(token_str.as_str()) };

    let issues =
        intake::github::fetch_issues(&cfg.github_owner, &cfg.github_repo, token_opt).await?;

    let repo_full = format!("{}/{}", cfg.github_owner, cfg.github_repo);
    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;

    for (gh_number, mut payload) in issues {
        payload.project_id = cfg.github_project_id.clone();

        let already: Option<(i64,)> = sqlx::query_as(
            "SELECT github_number FROM github_synced_issues WHERE github_number=? AND repo_full_name=?",
        )
        .bind(gh_number)
        .bind(&repo_full)
        .fetch_optional(&db)
        .await
        .map_err(|e| e.to_string())?;

        if already.is_some() {
            skipped += 1;
            continue;
        }

        match gateway::receive(&db, &job_tx, &app, payload).await {
            Ok(issue) => {
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO github_synced_issues
                     (github_number, repo_full_name, issue_id) VALUES (?, ?, ?)",
                )
                .bind(gh_number)
                .bind(&repo_full)
                .bind(&issue.id)
                .execute(&db)
                .await;
                imported += 1;
            }
            Err(_) => errors += 1,
        }
    }

    sqlx::query(
        "UPDATE intake_configs SET github_last_sync=datetime('now'), updated_at=datetime('now') WHERE id='singleton'",
    )
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(SyncResult { imported, skipped, errors })
}

// ── Code scan ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_code_scan(
    project_id: String,
    scan_types: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();

    let project = sqlx::query_as::<_, crate::models::project::Project>(
        "SELECT * FROM projects WHERE id=?",
    )
    .bind(&project_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "项目不存在".to_string())?;

    if project.repo_path.is_empty() {
        return Err("项目未配置仓库路径".to_string());
    }
    let repo_path = project.repo_path.clone();

    let do_todo = scan_types.is_empty() || scan_types.iter().any(|t| t == "todo");
    let do_cargo = scan_types.is_empty() || scan_types.iter().any(|t| t == "cargo_audit");
    let do_npm = scan_types.is_empty() || scan_types.iter().any(|t| t == "npm_audit");

    let mut all_payloads: Vec<IntakePayload> = vec![];
    if do_todo {
        all_payloads.extend(intake::scanner::scan_todos(&project_id, &repo_path).await);
    }
    if do_cargo {
        all_payloads.extend(intake::scanner::scan_cargo_audit(&project_id, &repo_path).await);
    }
    if do_npm {
        all_payloads.extend(intake::scanner::scan_npm_audit(&project_id, &repo_path).await);
    }

    let found = all_payloads.len() as u32;
    let mut new_issues = 0u32;
    for payload in all_payloads {
        if gateway::receive(&db, &job_tx, &app, payload).await.is_ok() {
            new_issues += 1;
        }
    }

    Ok(ScanResult { found, new_issues })
}

// ── Bulk import ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn bulk_import_issues(
    project_id: String,
    format: String,
    content: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BulkResult, String> {
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();

    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&db)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err("项目不存在".to_string());
    }

    let payloads = intake::bulk::parse(&project_id, &format, &content)?;
    let total = payloads.len() as u32;
    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = vec![];

    for payload in payloads {
        if payload.title.trim().is_empty() {
            skipped += 1;
            continue;
        }
        match gateway::receive(&db, &job_tx, &app, payload).await {
            Ok(_) => imported += 1,
            Err(e) => {
                if errors.len() < 20 {
                    errors.push(e);
                }
            }
        }
    }

    Ok(BulkResult { total, imported, skipped, errors })
}

// ── Submit from conversation artifact ───────────────────────────────────────

#[tauri::command]
pub async fn submit_from_artifact(
    payload: CreateIssue,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Issue, String> {
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();
    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: "conversation".to_string(),
        source_ref: payload.source_ref,
    };
    gateway::receive(&db, &job_tx, &app, intake).await
}
