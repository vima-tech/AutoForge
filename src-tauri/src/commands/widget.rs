use crate::models::intake::IntakeConfig;
use crate::models::widget::WidgetToken;
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

/// 生成一个新的 widget token 字符串（122 bit 随机，足够作公开接入凭据）。
fn new_token() -> String {
    format!("wgt_{}", Uuid::new_v4().simple())
}

/// 取该项目某模式下可用的接入 token：优先复用最近一个 enabled 且未过期的，否则新建一个。
/// mode="triage" 供反馈 widget；mode="flow" 供项目「需求入口」的 webhook 接入。
async fn ensure_token(
    db: &crate::db::Db,
    project_id: &str,
    mode: &str,
    label: &str,
) -> Result<WidgetToken, String> {
    let existing = sqlx::query_as::<_, WidgetToken>(
        "SELECT * FROM widget_tokens
         WHERE project_id = ? AND mode = ? AND enabled = 1
           AND (expires_at IS NULL OR expires_at > datetime('now'))
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(mode)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(t) = existing {
        return Ok(t);
    }
    insert_widget_token(db, project_id, label, mode).await
}

async fn insert_widget_token(
    db: &crate::db::Db,
    project_id: &str,
    label: &str,
    mode: &str,
) -> Result<WidgetToken, String> {
    let id = Uuid::new_v4().to_string();
    let token = new_token();
    sqlx::query(
        "INSERT INTO widget_tokens (id, project_id, token, label, enabled, mode)
         VALUES (?, ?, ?, ?, 1, ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&token)
    .bind(label)
    .bind(mode)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, WidgetToken>("SELECT * FROM widget_tokens WHERE id=?")
        .bind(&id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

/// 列出某项目的所有接入 token（管理用）。
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

/// 为项目创建一个新的 widget（triage）token。
#[tauri::command]
pub async fn create_widget_token(
    project_id: String,
    label: Option<String>,
    state: State<'_, AppState>,
) -> Result<WidgetToken, String> {
    let label = label.unwrap_or_default();
    insert_widget_token(&state.db, &project_id, &label, "triage").await
}

/// 取项目的 webhook 接入 token（mode=flow，无则自动签发）。
/// 这是「需求入口」里展示给外部集成用的 Bearer 凭证，命中即自动进分析队列。
#[tauri::command]
pub async fn get_project_webhook_token(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<WidgetToken, String> {
    ensure_token(&state.db, &project_id, "flow", "需求入口").await
}

/// 轮换项目的 webhook 接入 token：吊销现有所有 enabled 的 flow token，签发一个新的。
/// 旧 token 立即失效（用于泄露后重置）。
#[tauri::command]
pub async fn regenerate_project_webhook_token(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<WidgetToken, String> {
    sqlx::query(
        "UPDATE widget_tokens SET enabled = 0 WHERE project_id = ? AND mode = 'flow' AND enabled = 1",
    )
    .bind(&project_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    insert_widget_token(&state.db, &project_id, "需求入口", "flow").await
}

/// 启用/吊销一个接入 token（吊销即 enabled=0，立即失效，不影响其它 token）。
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
    let token = ensure_token(&state.db, &project_id, "triage", "默认").await?.token;
    let base = format!("http://localhost:{port}");
    Ok(format!(
        "<script src=\"{base}/widget.js\"\n        data-endpoint=\"{base}\"\n        data-project-id=\"{project_id}\"\n        data-api-key=\"{token}\"></script>"
    ))
}
