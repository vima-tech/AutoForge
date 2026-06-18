use crate::models::notify::{NotifyChannel, NotifyChannelView};
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_notify_channels(
    state: State<'_, AppState>,
) -> Result<Vec<NotifyChannelView>, String> {
    let rows = sqlx::query_as::<_, NotifyChannel>(
        "SELECT * FROM notify_channels ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(NotifyChannelView::from).collect())
}

#[derive(Deserialize)]
pub struct NotifyChannelPayload {
    pub name: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub events: String,
    /// 签名密钥 / Bearer token；空字符串表示不设置（更新时表示保留原值）。
    #[serde(default)]
    pub secret: String,
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
) -> Result<NotifyChannelView, String> {
    let id = Uuid::new_v4().to_string();
    let secret = if payload.secret.trim().is_empty() {
        String::new()
    } else {
        crate::core::secrets::encrypt_field(&payload.secret)?
    };
    sqlx::query(
        "INSERT INTO notify_channels (id, name, kind, target, events, secret, enabled)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.kind)
    .bind(&payload.target)
    .bind(&payload.events)
    .bind(&secret)
    .bind(payload.enabled)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    fetch_view(&state, &id).await
}

#[tauri::command]
pub async fn update_notify_channel(
    id: String,
    payload: NotifyChannelPayload,
    state: State<'_, AppState>,
) -> Result<NotifyChannelView, String> {
    if payload.secret.trim().is_empty() {
        // secret 留空 = 保留原值，不动 secret 列。
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
    } else {
        let secret = crate::core::secrets::encrypt_field(&payload.secret)?;
        sqlx::query(
            "UPDATE notify_channels SET name=?, kind=?, target=?, events=?, secret=?, enabled=? WHERE id=?",
        )
        .bind(&payload.name)
        .bind(&payload.kind)
        .bind(&payload.target)
        .bind(&payload.events)
        .bind(&secret)
        .bind(payload.enabled)
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    fetch_view(&state, &id).await
}

async fn fetch_view(state: &State<'_, AppState>, id: &str) -> Result<NotifyChannelView, String> {
    let row = sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels WHERE id=?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(NotifyChannelView::from(row))
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
pub async fn test_notify_channel(id: String, state: State<'_, AppState>) -> Result<(), String> {
    crate::core::notify::send_test(&id, &state.db).await
}

/// 微信 ClawBot 扫码绑定第 1 步：申请二维码。
#[tauri::command]
pub async fn clawbot_start_login() -> Result<crate::core::notify::ClawbotQrStart, String> {
    crate::core::notify::clawbot_qr_start().await
}

/// 微信 ClawBot 扫码绑定第 2 步：轮询扫码状态（前端循环调用）。
#[tauri::command]
pub async fn clawbot_poll_login(
    qrcode: String,
    base_url: String,
    verify_code: Option<String>,
) -> Result<crate::core::notify::ClawbotQrPoll, String> {
    crate::core::notify::clawbot_qr_poll(&qrcode, &base_url, verify_code.as_deref()).await
}
