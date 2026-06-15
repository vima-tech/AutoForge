use crate::models::intake::IntakeConfig;
use crate::state::AppState;
use tauri::State;

/// Return the `<script>` embed snippet for the M10 feedback widget for a project.
#[tauri::command]
pub async fn get_widget_snippet(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cfg = sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let (port, token) = cfg
        .map(|c| (c.webhook_port, c.webhook_token))
        .unwrap_or((27182, String::new()));
    let base = format!("http://localhost:{port}");
    Ok(format!(
        "<script src=\"{base}/widget.js\"\n        data-endpoint=\"{base}\"\n        data-project-id=\"{project_id}\"\n        data-api-key=\"{token}\"></script>"
    ))
}
