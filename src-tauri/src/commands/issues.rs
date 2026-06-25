use crate::commands::attachments_common::{
    attachment_path_from_rel, sanitize_file_name, validate_attachment, MAX_ATTACHMENT_BYTES,
};
use crate::intake::{gateway, IntakeMode, IntakePayload};
use crate::models::issue::{CreateIssue, Issue, IssueAnalysis};
use crate::models::issue_attachment::{IssueAttachment, IssueAttachmentUpload};
use crate::models::job::JobPayload;
use crate::state::AppState;
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub async fn list_issues(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    if let Some(pid) = project_id {
        sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, Issue>("SELECT * FROM issues ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_issue(id: String, state: State<'_, AppState>) -> Result<Option<Issue>, String> {
    sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 一页需求 + 该筛选条件下的总数。前端「全量需求总账」滚动动态加载用，
/// 避免需求量增长后一次性把全部需求拉进内存/渲染。
#[derive(serde::Serialize)]
pub struct IssuePage {
    pub items: Vec<Issue>,
    pub total: i64,
}

/// 分页查询需求的过滤参数聚合体，收拢 list_issues_page 的多个筛选字段，
/// 使 Tauri command 签名保持在 clippy::too_many_arguments 上限（7）内。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIssuesQuery {
    pub project_id: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub exclude_merged: Option<bool>,
    pub limit: i64,
    pub offset: i64,
    pub sort_asc: Option<bool>,
}

/// 分页查询需求：按项目 + 状态 + 关键字（标题/编号）过滤，按 updated_at 倒序。
/// status 为空或 "all" 表示不过滤状态；search 为空表示不过滤关键字。
/// exclude_merged 为 true 时排除「已合并」需求（与功能审计页「显示已合并需求」开关共享，默认隐藏）；
/// 当显式按 merged 状态筛选时该排除自然失效（status 优先）。
#[tauri::command]
pub async fn list_issues_page(
    query: ListIssuesQuery,
    state: State<'_, AppState>,
) -> Result<IssuePage, String> {
    use sqlx::{QueryBuilder, Sqlite};
    let ListIssuesQuery { project_id, status, search, exclude_merged, limit, offset, sort_asc } = query;
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let status = status.filter(|s| !s.is_empty() && s != "all");
    // 仅当未显式筛选 merged 时才生效（显式选 merged 说明用户主动想看，不再隐藏）。
    let exclude_merged =
        exclude_merged.unwrap_or(false) && status.as_deref() != Some("merged");
    // LIKE 通配串作为局部变量，确保在两次 query 构建期间存活。
    let like = search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{}%", s));

    let total: i64 = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        qb.build_query_scalar()
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let items = {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        // 统一按创建时间排序（updated_at 会随执行/重分析变动，跨页不稳定）。
        // 默认正序：旧需求置前，避免问题积压长期无人处理。
        qb.push(if sort_asc.unwrap_or(true) {
            " ORDER BY created_at ASC LIMIT "
        } else {
            " ORDER BY created_at DESC LIMIT "
        })
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
        qb.build_query_as::<Issue>()
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(IssuePage { items, total })
}

/// 某项目下出现过的所有状态（去重），用于总账的状态筛选 chip——
/// 不依赖已加载的那一页，仍能列出全部可筛选状态。
#[tauri::command]
pub async fn list_issue_statuses(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT DISTINCT status FROM issues WHERE project_id=?")
        .bind(&project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 按状态集合取需求（完整字段），用于「需求审核」队列等有界子集，
/// 替代为拿少量在产需求而全量加载 issues。
#[tauri::command]
pub async fn list_issues_by_statuses(
    project_id: String,
    statuses: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if statuses.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE project_id = ");
    qb.push_bind(&project_id).push(" AND status IN (");
    let mut sep = qb.separated(", ");
    for s in &statuses {
        sep.push_bind(s);
    }
    qb.push(") ORDER BY created_at DESC");
    qb.build_query_as::<Issue>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 需求标题（轻量），按 id 批量取，用于变更请求列表解析标题，
/// 无需把全部需求载入内存。
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct IssueTitle {
    pub id: String,
    pub title: String,
}

#[tauri::command]
pub async fn list_issue_titles(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<IssueTitle>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT id, title FROM issues WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id);
    }
    qb.push(")");
    qb.build_query_as::<IssueTitle>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_issue_analysis(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Option<IssueAnalysis>, String> {
    sqlx::query_as::<_, IssueAnalysis>("SELECT * FROM issue_analyses WHERE issue_id=?")
        .bind(&issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn submit_issue(
    payload: CreateIssue,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Issue, String> {
    let mode = IntakeMode::from_opt(payload.mode.as_deref());
    let has_bug = payload.repro_steps.is_some()
        || payload.environment.is_some()
        || payload.expected.is_some()
        || payload.actual.is_some();
    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: payload.source_type.unwrap_or_else(|| "manual".to_string()),
        source_ref: payload.source_ref,
    };
    let issue = gateway::receive(&state.db, &state.job_tx, &app, intake, mode).await?;

    // Bug 载体字段单独落库（保持 IntakePayload 与六通道不变）。
    if has_bug {
        sqlx::query(
            "UPDATE issues SET repro_steps=?, environment=?, expected=?, actual=? WHERE id=?",
        )
        .bind(&payload.repro_steps)
        .bind(&payload.environment)
        .bind(&payload.expected)
        .bind(&payload.actual)
        .bind(&issue.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        return sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&issue.id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }
    Ok(issue)
}

/// Re-run requirement analysis for an issue whose analysis failed (or is stuck at
/// review 1). Resets the issue to `pending_analysis` and re-enqueues the analysis
/// job under a UNIQUE idempotency key (`analysis:<id>:retry:<uuid>`). The stable
/// `analysis:<id>` key would silently de-duplicate against an already-`completed`
/// job row — `enqueue` only re-dispatches a `failed` row — leaving the issue parked
/// at `pending_analysis` forever. A unique key always dispatches, so a transient LLM
/// failure (or a re-analysis of an already-analyzed issue) recovers in one click.
/// The status-guarded UPDATE above already idempotently blocks a double-click.
#[tauri::command]
pub async fn retry_analysis(
    issue_id: String,
    _app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let updated = sqlx::query(
        "UPDATE issues SET status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('analysis_failed', 'pending_issue_review')",
    )
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「分析失败」或「待需求审核」状态的需求可重新分析".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 需求审核 补充意见重评：管理员在「待需求审核」阶段提交补充意见，需求带着该意见
/// 重新分析，再回到需求审核。意见落库到 issues.review_feedback（一次性，被分析任务
/// 消费后清空）。意见为人工输入，入库前过 has_obvious_injection 防注入。
#[tauri::command]
pub async fn reanalyze_with_feedback(
    issue_id: String,
    feedback: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let feedback = feedback.trim().to_string();
    if feedback.is_empty() {
        return Err("补充意见不能为空".to_string());
    }
    if crate::core::security::has_obvious_injection(&feedback) {
        return Err("补充意见包含可疑指令，已拦截".to_string());
    }
    // 仅「待需求审核」或「分析失败」状态可带意见重评，避免影响在产/已完结需求。
    let updated = sqlx::query(
        "UPDATE issues SET review_feedback=?, status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('pending_issue_review', 'analysis_failed')",
    )
    .bind(&feedback)
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「待需求审核」或「分析失败」状态的需求可提交补充意见重新评估".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        // 唯一 key：稳定的 analysis:<id> 会被 enqueue 按已 completed 的旧 job 行去重而
        // 不再派发，需求会永远停在 pending_analysis。带意见重评必须真正重跑。
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 列出某 CR 的测试遥测记录（review_2 合并前自动测试结果）。
#[tauri::command]
pub async fn list_cr_test_runs(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::test_run::CrTestRun>, String> {
    sqlx::query_as::<_, crate::models::test_run::CrTestRun>(
        "SELECT * FROM cr_test_runs WHERE cr_id=? ORDER BY run_at DESC",
    )
    .bind(&cr_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 人审改 AI 生成的验收标准（acceptance_json，JSON 数组字符串）。
#[tauri::command]
pub async fn update_issue_acceptance(
    issue_id: String,
    acceptance_json: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 校验是合法 JSON，避免写入脏数据破坏后续解析。
    serde_json::from_str::<serde_json::Value>(&acceptance_json)
        .map_err(|e| format!("验收标准 JSON 非法：{}", e))?;
    sqlx::query("UPDATE issues SET acceptance_json=?, updated_at=datetime('now') WHERE id=?")
        .bind(&acceptance_json)
        .bind(&issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 需求附件（issue_attachments）──────────────────────────────────────────────
// 镜像会议室附件（conversations.rs::import_attachment），存储基目录用
// attachments_base()/issues/<issue_id>/，复用 attachments_common 的安全白名单。
// 图片附件可经 supports_vision 的分析 Agent 内联识别（tasks/analysis.rs）。

async fn load_issue_attachment(
    db: &crate::db::Db,
    attachment_id: &str,
) -> Result<IssueAttachment, String> {
    sqlx::query_as::<_, IssueAttachment>("SELECT * FROM issue_attachments WHERE id=?")
        .bind(attachment_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "附件不存在".to_string())
}

#[tauri::command]
pub async fn import_issue_attachment(
    payload: IssueAttachmentUpload,
    state: State<'_, AppState>,
) -> Result<IssueAttachment, String> {
    let issue_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM issues WHERE id=?")
        .bind(&payload.issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if issue_exists.is_none() {
        return Err(format!("issue {} not found", payload.issue_id));
    }

    if payload.data_base64.len() > (MAX_ATTACHMENT_BYTES * 4 / 3 + 16) {
        return Err("附件超过 10 MB 上限".to_string());
    }

    let bytes = general_purpose::STANDARD
        .decode(payload.data_base64.as_bytes())
        .map_err(|_| "附件内容不是有效的 base64".to_string())?;
    if bytes.is_empty() {
        return Err("附件不能为空".to_string());
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err("附件超过 10 MB 上限".to_string());
    }

    let clean_name = sanitize_file_name(&payload.file_name);
    let policy = validate_attachment(&clean_name, payload.mime_hint.as_str(), &bytes)?;
    let attachment_id = Uuid::new_v4().to_string();
    let stored_name = format!("{}.{}", attachment_id, policy.ext);
    // rel_path 含 `issues/` 段，使 attachment_path_from_rel（= attachments_base()/rel_path）
    // 与下方落盘目录一致。
    let rel_path = format!("issues/{}/{}", payload.issue_id, stored_name);
    let issue_dir = PathBuf::from(crate::state::attachments_base())
        .join("issues")
        .join(&payload.issue_id);
    let file_path = issue_dir.join(&stored_name);

    tokio::fs::create_dir_all(&issue_dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| e.to_string())?;

    let sha256 = hex::encode(Sha256::digest(&bytes));
    let insert_result = sqlx::query(
        "INSERT INTO issue_attachments
         (id, issue_id, original_name, stored_name, rel_path, mime, kind, size_bytes, sha256)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&attachment_id)
    .bind(&payload.issue_id)
    .bind(&clean_name)
    .bind(&stored_name)
    .bind(&rel_path)
    .bind(policy.mime)
    .bind(policy.kind)
    .bind(bytes.len() as i64)
    .bind(&sha256)
    .execute(&state.db)
    .await;

    if let Err(e) = insert_result {
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err(e.to_string());
    }

    load_issue_attachment(&state.db, &attachment_id).await
}

#[tauri::command]
pub async fn list_issue_attachments(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<IssueAttachment>, String> {
    sqlx::query_as::<_, IssueAttachment>(
        "SELECT * FROM issue_attachments WHERE issue_id=? ORDER BY created_at ASC",
    )
    .bind(&issue_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn issue_attachment_data_url(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    if attachment.kind != "image" {
        return Err("只有图片附件支持内联预览".to_string());
    }
    if attachment.size_bytes as usize > MAX_ATTACHMENT_BYTES {
        return Err("图片超过预览大小上限".to_string());
    }
    let path = attachment_path_from_rel(&attachment.rel_path)?;
    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{};base64,{}",
        attachment.mime,
        general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub async fn open_issue_attachment(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    let path = attachment_path_from_rel(&attachment.rel_path)?;
    if !path.exists() {
        return Err("附件文件不存在或已被移除".to_string());
    }

    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    cmd.arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开附件：{}", e))
}

#[tauri::command]
pub async fn delete_issue_attachment(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    if let Ok(path) = attachment_path_from_rel(&attachment.rel_path) {
        let _ = tokio::fs::remove_file(&path).await;
    }
    sqlx::query("DELETE FROM issue_attachments WHERE id=?")
        .bind(&attachment_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
