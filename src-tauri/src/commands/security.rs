use crate::models::security_audit::SecurityAudit;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_security_audits(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SecurityAudit>, String> {
    match project_id {
        Some(pid) => sqlx::query_as::<_, SecurityAudit>(
            "SELECT * FROM security_audits WHERE project_id=? ORDER BY started_at DESC LIMIT 200",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        None => sqlx::query_as::<_, SecurityAudit>(
            "SELECT * FROM security_audits ORDER BY started_at DESC LIMIT 200",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}
