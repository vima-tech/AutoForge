use crate::models::intake::IntakeConfig;
use crate::models::widget::WidgetToken;
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

/// 生成一个新的 widget token 字符串（122 bit 随机，足够作公开接入凭据）。
fn new_token() -> String {
    format!("wgt_{}", Uuid::new_v4().simple())
}

/// 取该项目可用的 widget token：优先复用最近一个 enabled 且未过期的，否则新建一个。
/// 让「生成嵌入代码」一键即用，同时把接入凭据从主 webhook_token 解耦出来。
async fn ensure_widget_token(
    db: &crate::db::Db,
    project_id: &str,
) -> Result<WidgetToken, String> {
    let existing = sqlx::query_as::<_, WidgetToken>(
        "SELECT * FROM widget_tokens
         WHERE project_id = ? AND enabled = 1
           AND (expires_at IS NULL OR expires_at > datetime('now'))
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(t) = existing {
        return Ok(t);
    }
    insert_widget_token(db, project_id, "默认").await
}

async fn insert_widget_token(
    db: &crate::db::Db,
    project_id: &str,
    label: &str,
) -> Result<WidgetToken, String> {
    let id = Uuid::new_v4().to_string();
    let token = new_token();
    sqlx::query(
        "INSERT INTO widget_tokens (id, project_id, token, label, enabled)
         VALUES (?, ?, ?, ?, 1)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&token)
    .bind(label)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, WidgetToken>("SELECT * FROM widget_tokens WHERE id=?")
        .bind(&id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

/// 列出某项目的所有 widget token（管理用）。
#[tauri::command]
pub async fn list_widget_tokens(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WidgetToken>, String> {
    sqlx::query_as::<_, WidgetToken>(
        "SELECT * FROM widget_tokens WHERE project_id = ? ORDER BY created_at DESC",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 为项目创建一个新的 widget token。
#[tauri::command]
pub async fn create_widget_token(
    project_id: String,
    label: Option<String>,
    state: State<'_, AppState>,
) -> Result<WidgetToken, String> {
    let label = label.unwrap_or_default();
    insert_widget_token(&state.db, &project_id, &label).await
}

/// 启用/吊销一个 widget token（吊销即 enabled=0，立即失效，不影响主 token 与其它 token）。
#[tauri::command]
pub async fn set_widget_token_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query("UPDATE widget_tokens SET enabled = ? WHERE id = ?")
        .bind(enabled as i64)
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除一个 widget token。
#[tauri::command]
pub async fn delete_widget_token(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM widget_tokens WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Return the `<script>` embed snippet for the M10 feedback widget for a project.
/// 使用项目专属、可独立吊销的 widget token（无则自动创建），不再暴露主 webhook_token。
#[tauri::command]
pub async fn get_widget_snippet(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cfg = sqlx::query_as::<_, IntakeConfig>("SELECT * FROM intake_configs WHERE id='singleton'")
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let port = cfg.map(|c| c.webhook_port).unwrap_or(27182);
    let token = ensure_widget_token(&state.db, &project_id).await?.token;
    let base = format!("http://localhost:{port}");
    Ok(format!(
        "<script src=\"{base}/widget.js\"\n        data-endpoint=\"{base}\"\n        data-project-id=\"{project_id}\"\n        data-api-key=\"{token}\"></script>"
    ))
}
