use crate::intake::{gateway, IntakePayload};
use crate::models::issue::{CreateIssue, Issue, IssueAnalysis};
use crate::state::AppState;
use tauri::{AppHandle, State};

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
    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: payload.source_type.unwrap_or_else(|| "manual".to_string()),
        source_ref: payload.source_ref,
    };
    gateway::receive(&state.db, &state.job_tx, &app, intake).await
}
