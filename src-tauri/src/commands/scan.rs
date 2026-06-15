use crate::state::AppState;
use tauri::{AppHandle, State};

/// Manually trigger a proactive inspection scan for a project (design §6.2 mode B).
#[tauri::command]
pub async fn run_proactive_scan(
    project_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::tasks::scan::run_proactive(&state.db, &state.job_tx, &app, &project_id, "manual")
        .await
        .map_err(|e| e.to_string())
}
