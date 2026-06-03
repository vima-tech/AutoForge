use crate::core::{event, security};
use crate::models::issue::{CreateIssue, Issue, IssueAnalysis};
use crate::models::job::JobPayload;
use crate::state::AppState;
use crate::tasks::runner::enqueue;
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
    // Security check
    if security::has_obvious_injection(&payload.title) {
        return Err("标题包含可疑内容，提交被拒绝".to_string());
    }
    let desc = payload.description.as_deref().unwrap_or("");
    if security::has_obvious_injection(desc) {
        return Err("描述包含可疑内容，提交被拒绝".to_string());
    }

    let title = security::safe_truncate(&payload.title, 200);
    let description = security::safe_truncate(desc, 4000);

    // Fingerprint for dedup
    let fp = security::fingerprint(&title, &description);

    // Check for duplicates
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM issues WHERE fingerprint=? AND project_id=?")
            .bind(&fp)
            .bind(&payload.project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    if let Some((dup_id,)) = existing {
        return sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&dup_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }

    let id = Uuid::new_v4().to_string();
    let category = payload.category.unwrap_or_else(|| "Feature".to_string());
    let severity = payload.severity.unwrap_or_else(|| "medium".to_string());
    let source_type = payload.source_type.unwrap_or_else(|| "manual".to_string());

    sqlx::query(
        "INSERT INTO issues (id, project_id, source_type, title, description, category, severity, fingerprint)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&payload.project_id)
    .bind(&source_type)
    .bind(&title)
    .bind(&description)
    .bind(&category)
    .bind(&severity)
    .bind(&fp)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Enqueue analysis job
    let idem_key = format!("analysis:{}", id);
    let _ = enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        &idem_key,
        JobPayload::Analysis {
            issue_id: id.clone(),
        },
    )
    .await;

    let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    event::emit(
        &app,
        event::AppEvent::IssueCreated {
            issue_id: issue.id.clone(),
            project_id: issue.project_id.clone(),
        },
    );

    Ok(issue)
}
