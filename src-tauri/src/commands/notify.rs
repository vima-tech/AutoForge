use crate::models::notify::NotifyChannel;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_notify_channels(
    state: State<'_, AppState>,
) -> Result<Vec<NotifyChannel>, String> {
    sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct NotifyChannelPayload {
    pub name: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub events: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}
fn default_true() -> bool {
    true
}

#[tauri::command]
pub async fn create_notify_channel(
    payload: NotifyChannelPayload,
    state: State<'_, AppState>,
) -> Result<NotifyChannel, String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notify_channels (id, name, kind, target, events, enabled)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.kind)
    .bind(&payload.target)
    .bind(&payload.events)
    .bind(payload.enabled)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_notify_channel(
    id: String,
    payload: NotifyChannelPayload,
    state: State<'_, AppState>,
) -> Result<NotifyChannel, String> {
    sqlx::query(
        "UPDATE notify_channels SET name=?, kind=?, target=?, events=?, enabled=? WHERE id=?",
    )
    .bind(&payload.name)
    .bind(&payload.kind)
    .bind(&payload.target)
    .bind(&payload.events)
    .bind(payload.enabled)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_notify_channel(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM notify_channels WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn test_notify_channel(kind: String, target: String) -> Result<(), String> {
    crate::core::notify::send_test(&kind, &target).await
}
