use crate::intake::{self, gateway, IntakeMode, IntakePayload};
use crate::models::intake::{
    BulkResult, IntakeConfig, RefineResult, RejectResult, ScanResult, SyncResult,
    UpdateIntakeConfig, WebhookStatus,
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

    // 不再要求主 token：启用即起服务，入站请求按项目级 token 鉴权。
    if cfg.webhook_enabled {
        let port = cfg.webhook_port as u16;
        let db_clone = db.clone();
        let app_clone = app.clone();
        let new_handle = tokio::spawn(async move {
            if let Err(e) = intake::webhook::start(port, db_clone, job_tx, app_clone).await {
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

// ── Decide a requirement_draft inside a conversation card ────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct DecideDraftPayload {
    pub message_id: String,
    /// "confirm" | "reject"
    pub decision: String,
    /// 可选：草稿块在消息中的下标；缺省时取首个 requirement_draft 块。
    #[serde(default)]
    pub block_index: Option<usize>,
}

/// 在会议室对话 card 内直接「确认 / 拒绝」一条整理好的需求草稿，让整理环节在群聊里闭环。
/// - confirm：从草稿 `_meta` 经统一网关入流水线（Flow，含注入过滤 + 去重），并把该块标记
///   `decided="confirmed"`；
/// - reject：仅把该块标记 `decided="rejected"`，不入库。
/// 决策写回 `messages.content_json` 持久化，刷新后按钮不再重现、不可重复操作。
/// 命令保持薄包装，逻辑下沉到 `decide_requirement_draft_inner`（不含 Tauri 类型，事件除外）。
#[tauri::command]
pub async fn decide_requirement_draft(
    payload: DecideDraftPayload,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Issue>, String> {
    let db = state.db.clone();
    let job_tx = state.job_tx.clone();
    decide_requirement_draft_inner(&db, &job_tx, &app, payload).await
}

fn is_requirement_draft(v: &serde_json::Value) -> bool {
    v.get("t").and_then(|t| t.as_str()) == Some("artifact")
        && v.get("kind").and_then(|k| k.as_str()) == Some("requirement_draft")
}

async fn decide_requirement_draft_inner(
    db: &crate::db::Db,
    job_tx: &crate::tasks::runner::JobSender,
    app: &AppHandle,
    payload: DecideDraftPayload,
) -> Result<Option<Issue>, String> {
    let decision = payload.decision.trim();
    if decision != "confirm" && decision != "reject" {
        return Err("decision 必须是 confirm 或 reject".to_string());
    }

    let row: Option<(String,)> = sqlx::query_as("SELECT content_json FROM messages WHERE id=?")
        .bind(&payload.message_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?;
    let content_json = row.ok_or_else(|| "消息不存在".to_string())?.0;
    let mut blocks: Vec<serde_json::Value> =
        serde_json::from_str(&content_json).map_err(|e| format!("消息内容解析失败: {}", e))?;

    // 定位 requirement_draft 块：优先用前端给的下标，否则取首个匹配块。
    let idx = payload
        .block_index
        .filter(|&i| blocks.get(i).map(is_requirement_draft).unwrap_or(false))
        .or_else(|| blocks.iter().position(is_requirement_draft))
        .ok_or_else(|| "该消息中没有需求草稿".to_string())?;

    if let Some(d) = blocks[idx].get("decided").and_then(|v| v.as_str()) {
        let label = if d == "confirmed" { "确认" } else { "拒绝" };
        return Err(format!("该需求已{}，不能重复操作", label));
    }

    let mut issue = None;
    if decision == "confirm" {
        let meta = blocks[idx].get("_meta").cloned().unwrap_or_default();
        let pick = |key: &str| meta.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
        let project_id = pick("project_id").unwrap_or_default();
        if project_id.trim().is_empty() {
            return Err("需求草稿缺少项目信息，无法入库".to_string());
        }
        let title = pick("title")
            .or_else(|| blocks[idx].get("title").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        let description = pick("description")
            .or_else(|| blocks[idx].get("body").and_then(|v| v.as_str()).map(|s| s.to_string()));
        let intake = IntakePayload {
            project_id,
            title,
            description,
            category: pick("category"),
            severity: pick("severity"),
            source_type: "conversation".to_string(),
            source_ref: Some(payload.message_id.clone()),
        };
        issue = Some(gateway::receive(db, job_tx, app, intake, IntakeMode::Flow).await?);
    }

    let decided = if decision == "confirm" { "confirmed" } else { "rejected" };
    if let Some(obj) = blocks[idx].as_object_mut() {
        obj.insert("decided".to_string(), serde_json::json!(decided));
    }
    let new_json = serde_json::to_string(&blocks).map_err(|e| e.to_string())?;
    sqlx::query("UPDATE messages SET content_json=? WHERE id=?")
        .bind(&new_json)
        .bind(&payload.message_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;

    Ok(issue)
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

    // 批量整理引擎下沉到纯模块 `intake::triage`（切批 + 有界并发跑 triage Agent）。
    // DB 写入与事件发射仍在下方串行进行，对 Tauri 类型的依赖只留在命令体内（解耦铁律）。
    let outcomes = intake::triage::batch_triage(&state.db, loaded).await;

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
    if state.autosupply_running.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("已有一轮自喂料正在进行".into());
    }
    let cfg = crate::tasks::autosupply::AutosupplyConfig::load(&state.db).await;
    let s = crate::tasks::autosupply::run_cycle(
        &state.db,
        &state.job_tx,
        &app,
        &cfg,
        &state.autosupply_running,
    )
    .await;
    let produced = s.scanned + s.proposed;
    Ok(ScanResult {
        found: produced,
        // 前置整理去噪后，实际留在待整理池的净条数。
        new_issues: produced.saturating_sub(s.discarded),
    })
}

/// 查询当前是否正有一轮自喂料在跑。前端切页重挂载后据此恢复「运行中」回显——
/// 状态真源在后端，本地组件状态丢失也不影响。
#[tauri::command]
pub async fn autosupply_is_running(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state
        .autosupply_running
        .load(std::sync::atomic::Ordering::SeqCst))
}
