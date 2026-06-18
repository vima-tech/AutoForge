use super::{IntakeMode, IntakePayload};
use crate::core::{event, security};
use crate::db::Db;
use crate::models::issue::Issue;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use tauri::AppHandle;
use uuid::Uuid;

/// 统一需求接收网关：去重、安全检查、入库、入队分析
pub async fn receive(
    db: &Db,
    job_tx: &JobSender,
    app: &AppHandle,
    payload: IntakePayload,
    mode: IntakeMode,
) -> Result<Issue, String> {
    if security::has_obvious_injection(&payload.title) {
        return Err("标题包含可疑内容，提交被拒绝".to_string());
    }
    let desc = payload.description.as_deref().unwrap_or("");
    if security::has_obvious_injection(desc) {
        return Err("描述包含可疑内容，提交被拒绝".to_string());
    }

    let title = security::safe_truncate(&payload.title, 200);
    let description = security::safe_truncate(desc, 4000);

    let fp = security::fingerprint(&title, &description);

    // 去重：返回已存在的 Issue（幂等）
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM issues WHERE fingerprint=? AND project_id=?")
            .bind(&fp)
            .bind(&payload.project_id)
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())?;

    if let Some((dup_id,)) = existing {
        return sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&dup_id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string());
    }

    let id = Uuid::new_v4().to_string();
    let category = payload.category.unwrap_or_else(|| "Feature".to_string());
    let severity = payload.severity.unwrap_or_else(|| "medium".to_string());

    // triage 模式落「待整理池」(status='triage')，保留原始文本，且不自动入队分析。
    let status = mode.initial_status();
    let raw_capture = (mode == IntakeMode::Triage).then(|| description.clone());

    sqlx::query(
        "INSERT INTO issues
         (id, project_id, source_type, title, description, category, severity, status, fingerprint, source_ref, raw_capture)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.project_id)
    .bind(&payload.source_type)
    .bind(&title)
    .bind(&description)
    .bind(&category)
    .bind(&severity)
    .bind(status)
    .bind(&fp)
    .bind(&payload.source_ref)
    .bind(&raw_capture)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 仅 Flow 模式自动入队分析；triage 待整理池等人/triage Agent 处理后再转 Flow。
    if mode == IntakeMode::Flow {
        let idem_key = format!("analysis:{}", id);
        let _ = enqueue(
            db,
            job_tx,
            "analysis",
            &idem_key,
            JobPayload::Analysis { issue_id: id.clone() },
        )
        .await;
    }

    let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    event::emit(
        app,
        event::AppEvent::IssueCreated {
            issue_id: issue.id.clone(),
            project_id: issue.project_id.clone(),
        },
    );

    Ok(issue)
}
