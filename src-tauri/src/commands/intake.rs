use crate::intake::{self, gateway, IntakeMode, IntakePayload};
use crate::models::intake::{
    BulkResult, IntakeConfig, RefineResult, ScanResult, SyncResult, UpdateIntakeConfig,
    WebhookStatus,
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

        match gateway::receive(&db, &job_tx, &app, payload, IntakeMode::Flow).await {
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
        if gateway::receive(&db, &job_tx, &app, payload, IntakeMode::Flow).await.is_ok() {
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
    run_bulk_payloads(&db, &job_tx, &app, payloads).await
}

/// 从上传的文件（csv/xlsx/xls/ods）批量导入需求。
/// 文件内容以 base64 传入，按文件名后缀解析为载荷后走统一网关。
#[tauri::command]
pub async fn bulk_import_file(
    project_id: String,
    file_name: String,
    data_base64: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BulkResult, String> {
    use base64::{engine::general_purpose, Engine as _};

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

    let bytes = general_purpose::STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("base64 解码错误: {}", e))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err("文件过大（上限 10 MB）".to_string());
    }

    let payloads = intake::bulk::parse_file(&project_id, &file_name, &bytes)?;
    run_bulk_payloads(&db, &job_tx, &app, payloads).await
}

/// 共用：逐条把载荷送入网关并汇总结果。
async fn run_bulk_payloads(
    db: &crate::db::Db,
    job_tx: &crate::tasks::runner::JobSender,
    app: &AppHandle,
    payloads: Vec<IntakePayload>,
) -> Result<BulkResult, String> {
    let total = payloads.len() as u32;
    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = vec![];

    for payload in payloads {
        if payload.title.trim().is_empty() {
            skipped += 1;
            continue;
        }
        match gateway::receive(db, job_tx, app, payload, IntakeMode::Flow).await {
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

/// 导出批量导入模板（csv/xlsx），写入系统下载目录并在文件管理器中定位。
/// 返回写入的文件绝对路径。
#[tauri::command]
pub async fn export_bulk_template(format: String, app: AppHandle) -> Result<String, String> {
    use tauri::Manager;
    use tauri_plugin_opener::OpenerExt;

    let (file_name, _mime, bytes) = intake::bulk::template_bytes(&format)?;

    // 优先写到下载目录，否则退化到临时目录。
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir());
    let dest = dir.join(&file_name);

    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("写入模板失败: {}", e))?;

    let dest_str = dest.to_string_lossy().to_string();
    // 在文件管理器中定位（失败不致命）。
    let _ = app.opener().reveal_item_in_dir(&dest_str);

    Ok(dest_str)
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
    gateway::receive(&db, &job_tx, &app, intake, IntakeMode::Flow).await
}

// ── Triage（待整理池）────────────────────────────────────────────────────────

/// 列出待整理池条目（status='triage'）。可按项目过滤。
#[tauri::command]
pub async fn list_triage_issues(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    let res = if let Some(pid) = project_id {
        sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE status='triage' AND project_id=? ORDER BY created_at DESC",
        )
        .bind(pid)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE status='triage' ORDER BY created_at DESC",
        )
        .fetch_all(&state.db)
        .await
    };
    res.map_err(|e| e.to_string())
}

/// 整理若干待整理碎片：triage Agent 补全/分类 → 转入正常流水线（pending_analysis）。
/// 判为噪音的直接丢弃。**人工触发**，是 triage 池流向流水线的唯一闸门。
#[tauri::command]
pub async fn refine_triage(
    issue_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RefineResult, String> {
    let mut refined = 0u32;
    let mut discarded = 0u32;
    let mut errors = 0u32;

    for id in issue_ids {
        let issue = match sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE id=? AND status='triage'",
        )
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        {
            Some(i) => i,
            None => continue,
        };

        let raw = issue
            .raw_capture
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("{}\n{}", issue.title, issue.description));

        let out = match crate::agents::llm::run_system_role_text(
            &state.db,
            "triage",
            &raw,
            None,
            Some(&issue.project_id),
            None,
        )
        .await
        {
            Ok(t) => t,
            Err(_) => {
                errors += 1;
                continue;
            }
        };

        let Some(p) = parse_triage_json(&out) else {
            errors += 1;
            continue;
        };

        if p.is_noise {
            let _ = sqlx::query("DELETE FROM issues WHERE id=? AND status='triage'")
                .bind(&id)
                .execute(&state.db)
                .await;
            discarded += 1;
            continue;
        }

        sqlx::query(
            "UPDATE issues SET title=?, category=?, severity=?, description=?,
             status='pending_analysis', updated_at=datetime('now')
             WHERE id=? AND status='triage'",
        )
        .bind(&p.title)
        .bind(&p.category)
        .bind(&p.severity)
        .bind(&p.description)
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        let idem = format!("analysis:{}", id);
        let _ = crate::tasks::runner::enqueue(
            &state.db,
            &state.job_tx,
            "analysis",
            &idem,
            crate::models::job::JobPayload::Analysis { issue_id: id.clone() },
        )
        .await;
        crate::core::event::emit(
            &app,
            crate::core::event::AppEvent::IssueCreated {
                issue_id: id.clone(),
                project_id: issue.project_id.clone(),
            },
        );
        refined += 1;
    }

    Ok(RefineResult { refined, discarded, errors })
}

/// 直接丢弃一条待整理碎片（人工判定为噪音/无效）。
#[tauri::command]
pub async fn discard_triage(issue_id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM issues WHERE id=? AND status='triage'")
        .bind(&issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 工厂自喂料手动触发 ────────────────────────────────────────────────────────

/// 立即对单个项目跑 proposer，提议入 triage 池（安全护栏：永远 Triage）。
#[tauri::command]
pub async fn run_proposer(
    project_id: String,
    max: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let payloads = crate::intake::proposer::propose(
        &state.db,
        &project_id,
        max.unwrap_or(8) as usize,
    )
    .await
    .map_err(|e| e.to_string())?;
    let found = payloads.len() as u32;
    let mut new_issues = 0u32;
    for p in payloads {
        if gateway::receive(&state.db, &state.job_tx, &app, p, IntakeMode::Triage)
            .await
            .is_ok()
        {
            new_issues += 1;
        }
    }
    Ok(ScanResult { found, new_issues })
}

/// 立即跑一轮完整自喂料（扫描 + proposer），全部入 triage 池。
#[tauri::command]
pub async fn run_autosupply_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanResult, String> {
    let cfg = crate::tasks::autosupply::AutosupplyConfig::load(&state.db).await;
    let (scanned, proposed) =
        crate::tasks::autosupply::run_cycle(&state.db, &state.job_tx, &app, &cfg).await;
    Ok(ScanResult {
        found: scanned + proposed,
        new_issues: scanned + proposed,
    })
}

struct TriageParsed {
    title: String,
    category: String,
    severity: String,
    description: String,
    is_noise: bool,
}

/// 解析 triage Agent 输出的 JSON 对象（容忍 ```json 围栏与前后噪声）。
fn parse_triage_json(out: &str) -> Option<TriageParsed> {
    let start = out.find('{')?;
    let end = out.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out[start..=end]).ok()?;
    let is_noise = v.get("is_noise").and_then(|x| x.as_bool()).unwrap_or(false);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if title.is_empty() && !is_noise {
        return None;
    }
    Some(TriageParsed {
        title,
        category: v.get("category").and_then(|x| x.as_str()).unwrap_or("Feature").to_string(),
        severity: v.get("severity").and_then(|x| x.as_str()).unwrap_or("medium").to_string(),
        description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        is_noise,
    })
}
