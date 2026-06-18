use crate::intake::{self, gateway, IntakeMode, IntakePayload};
use crate::models::intake::{
    BulkResult, IntakeConfig, RefineResult, RejectResult, ScanResult, SyncResult,
    UpdateIntakeConfig, WebhookStatus,
};
use crate::models::issue::{CreateIssue, Issue};
use crate::state::AppState;
use futures::StreamExt;
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

    // scan_types 为空 = 自动模式：todo 总跑，安全扫描器按栈画像自动启用
    // （cargo_audit/npm_audit/pip_audit/govulncheck）。显式指定时按 token 各自启用。
    let auto = scan_types.is_empty();
    let detected: Vec<&str> = if auto {
        crate::core::stack::security_scanners(std::path::Path::new(&repo_path))
    } else {
        vec![]
    };
    let enabled = |id: &str| {
        if auto {
            detected.contains(&id)
        } else {
            scan_types.iter().any(|t| t == id)
        }
    };
    let do_todo = auto || scan_types.iter().any(|t| t == "todo");

    let mut all_payloads: Vec<IntakePayload> = vec![];
    if do_todo {
        all_payloads.extend(intake::scanner::scan_todos(&project_id, &repo_path).await);
    }
    if enabled("cargo_audit") {
        all_payloads.extend(intake::scanner::scan_cargo_audit(&project_id, &repo_path).await);
    }
    if enabled("npm_audit") {
        all_payloads.extend(intake::scanner::scan_npm_audit(&project_id, &repo_path).await);
    }
    if enabled("pip_audit") {
        all_payloads.extend(intake::scanner::scan_pip_audit(&project_id, &repo_path).await);
    }
    if enabled("govulncheck") {
        all_payloads.extend(intake::scanner::scan_govulncheck(&project_id, &repo_path).await);
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

    // 先串行（快速 DB 读）取出仍处于 triage 的碎片，跳过已不存在/状态已变的。
    let mut loaded = Vec::new();
    for id in issue_ids {
        if let Some(issue) = sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE id=? AND status='triage'",
        )
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        {
            loaded.push(issue);
        }
    }

    // triage LLM 调用是整批耗时的瓶颈。两级优化：
    //   1) 按项目 + token 预算切成小批，每批一次 LLM 请求整理多条（省往返/省重复系统提示）；
    //   2) 批之间仍有界并发；批响应漏返的条目自动回退单条整理，绝不丢需求。
    // DB 写入与事件发射仍在下方串行进行，对 Tauri 类型的依赖只留在命令体内（解耦铁律）。
    const BATCH_CONCURRENCY: usize = 4;
    let batches = group_for_triage(loaded);
    let nested: Vec<Vec<(Issue, Option<TriageParsed>)>> = futures::stream::iter(
        batches.into_iter().map(|batch| {
            let db = &state.db;
            async move { refine_triage_batch(db, batch).await }
        }),
    )
    .buffer_unordered(BATCH_CONCURRENCY)
    .collect()
    .await;
    let outcomes: Vec<(Issue, Option<TriageParsed>)> = nested.into_iter().flatten().collect();

    for (issue, parsed) in outcomes {
        let id = issue.id.clone();
        // LLM 失败或解析失败：计为出错，跳过。
        let Some(p) = parsed else {
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

/// 批量拒绝需求（全量总账的「拒绝（删除/归档）」操作）。
///
/// 按状态采取安全语义：
/// - `triage` 碎片：尚未进入流水线、无下游数据，直接硬删除。
/// - 执行中 / 待合并 / 已合并：不动（避免与运行中的 worktree 任务或已落地结果脱节），计入 skipped。
/// - 其余（待审核/分析中/各类失败/等待）：软归档为 `rejected`，保留记录可回看、规避外键孤儿。
#[tauri::command]
pub async fn reject_issues(
    issue_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RejectResult, String> {
    let mut r = RejectResult { deleted: 0, rejected: 0, skipped: 0 };
    for id in issue_ids {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM issues WHERE id=?")
                .bind(&id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        let Some(status) = status else {
            r.skipped += 1;
            continue;
        };
        match status.as_str() {
            "triage" => {
                sqlx::query("DELETE FROM issues WHERE id=? AND status='triage'")
                    .bind(&id)
                    .execute(&state.db)
                    .await
                    .map_err(|e| e.to_string())?;
                r.deleted += 1;
            }
            // 运行中 / 已落地：不允许拒绝，避免脱节。
            "executing" | "building" | "running" | "pending_execution" | "pending_merge"
            | "merged" => {
                r.skipped += 1;
            }
            // 已拒绝：幂等跳过。
            "rejected" => {
                r.skipped += 1;
            }
            _ => {
                sqlx::query(
                    "UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?",
                )
                .bind(&id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
                r.rejected += 1;
            }
        }
    }
    Ok(r)
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

#[derive(Clone)]
struct TriageParsed {
    title: String,
    category: String,
    severity: String,
    description: String,
    is_noise: bool,
}

/// 从单个 JSON 对象抽取 triage 字段（容错缺省）。title 为空且非噪音视为无效。
fn triage_from_value(v: &serde_json::Value) -> Option<TriageParsed> {
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

/// 解析 triage Agent 输出的单个 JSON 对象（容忍 ```json 围栏与前后噪声）。
fn parse_triage_json(out: &str) -> Option<TriageParsed> {
    let start = out.find('{')?;
    let end = out.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out[start..=end]).ok()?;
    triage_from_value(&v)
}

/// 解析批量 triage 输出的 JSON 数组：返回 `idx -> TriageParsed` 映射。
/// 每个元素需带 `idx`（或 `index`/`id`）整数指向输入序号；缺失/坏元素被跳过，
/// 由上层对未命中的 idx 回退到单条整理，保证不丢需求。
fn parse_triage_batch(out: &str) -> std::collections::HashMap<usize, TriageParsed> {
    let mut map = std::collections::HashMap::new();
    let (Some(start), Some(end)) = (out.find('['), out.rfind(']')) else {
        return map;
    };
    if end <= start {
        return map;
    }
    let Ok(serde_json::Value::Array(arr)) =
        serde_json::from_str::<serde_json::Value>(&out[start..=end])
    else {
        return map;
    };
    for el in &arr {
        let idx = el
            .get("idx")
            .or_else(|| el.get("index"))
            .or_else(|| el.get("id"))
            .and_then(|x| x.as_u64());
        if let (Some(idx), Some(parsed)) = (idx, triage_from_value(el)) {
            map.insert(idx as usize, parsed);
        }
    }
    map
}

/// 取一条碎片用于整理的原始文本：优先 raw_capture，否则回退 title+description。
fn triage_raw(issue: &Issue) -> String {
    issue
        .raw_capture
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}\n{}", issue.title, issue.description))
}

/// 单条整理：调 triage Agent → 解析单 JSON 对象。失败返回 None（计为出错）。
async fn refine_triage_one(db: &crate::db::Db, issue: &Issue) -> Option<TriageParsed> {
    crate::agents::llm::run_system_role_text(
        db,
        "triage",
        &triage_raw(issue),
        None,
        Some(&issue.project_id),
        None,
    )
    .await
    .ok()
    .and_then(|out| parse_triage_json(&out))
}

/// 一个小批的整理：单条直接走单请求；多条拼成一次批请求（输出 JSON 数组），
/// 对批响应里缺失的 idx 逐条回退到单请求。返回与输入同序的 (Issue, 解析结果)。
async fn refine_triage_batch(
    db: &crate::db::Db,
    batch: Vec<Issue>,
) -> Vec<(Issue, Option<TriageParsed>)> {
    if batch.len() <= 1 {
        let mut out = Vec::with_capacity(batch.len());
        for issue in batch {
            let parsed = refine_triage_one(db, &issue).await;
            out.push((issue, parsed));
        }
        return out;
    }

    // 同批共用一次召回（按首条所在项目），系统提示词来自 triage 角色；
    // 这里用 user prompt 显式切到「批量数组」模式，覆盖单对象输出约束。
    let mut prompt = String::with_capacity(256);
    prompt.push_str(&format!(
        "本次为**批量整理模式**（忽略系统提示中「只输出一个对象」的约束）。下面是 {} 条待整理的原始碎片，每条以 [序号] 开头。\n\
         请对每条按同样的整理规则处理，输出一个**严格 JSON 数组**（不要 Markdown、不要解释文字），\n\
         数组每个元素字段：idx（整数，对应下面的序号）、title、category、severity、description、is_noise（含义同单条规则）。\n\
         务必为每个序号都返回恰好一个元素，只输出 JSON 数组。\n",
        batch.len()
    ));
    for (i, it) in batch.iter().enumerate() {
        prompt.push_str(&format!("\n[{}] {}\n", i, triage_raw(it)));
    }

    let project_id = batch[0].project_id.clone();
    let map = match crate::agents::llm::run_system_role_text(
        db, "triage", &prompt, None, Some(&project_id), None,
    )
    .await
    {
        Ok(out) => parse_triage_batch(&out),
        Err(_) => std::collections::HashMap::new(),
    };

    let mut results = Vec::with_capacity(batch.len());
    for (i, issue) in batch.into_iter().enumerate() {
        match map.get(&i) {
            Some(p) => results.push((issue, Some(p.clone()))),
            // 批响应漏了这条 → 回退单条整理，绝不丢需求。
            None => {
                let parsed = refine_triage_one(db, &issue).await;
                results.push((issue, parsed));
            }
        }
    }
    results
}

/// 按「项目边界 + 条数/字符预算」把碎片切成小批，控制单次请求体量，
/// 避免输出过长被截断（token 感知的粗粒度近似：字符数 ≈ token 的保守上界）。
fn group_for_triage(items: Vec<Issue>) -> Vec<Vec<Issue>> {
    const MAX_ITEMS: usize = 8;
    const MAX_CHARS: usize = 6000;
    let mut batches: Vec<Vec<Issue>> = Vec::new();
    let mut cur: Vec<Issue> = Vec::new();
    let mut cur_chars = 0usize;
    for it in items {
        let c = triage_raw(&it).chars().count();
        let cross_project = cur
            .first()
            .map(|f: &Issue| f.project_id != it.project_id)
            .unwrap_or(false);
        if !cur.is_empty()
            && (cross_project || cur.len() >= MAX_ITEMS || cur_chars + c > MAX_CHARS)
        {
            batches.push(std::mem::take(&mut cur));
            cur_chars = 0;
        }
        cur_chars += c;
        cur.push(it);
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    batches
}
