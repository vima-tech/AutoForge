use crate::models::intake::IntakeConfig;
use crate::models::widget::WidgetToken;
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

/// 生成一个新的 widget token 字符串（122 bit 随机，足够作公开接入凭据）。
fn new_token() -> String {
    format!("wgt_{}", Uuid::new_v4().simple())
}

/// 读取该项目某模式下当前可用（enabled 且未过期）的 token。
/// 排序加 `id` 兜底平局，保证读回确定（配合 0075 唯一索引，正常情况下也只会有一条）。
async fn fetch_enabled_token(
    db: &crate::db::Db,
    project_id: &str,
    mode: &str,
) -> Result<Option<WidgetToken>, String> {
    sqlx::query_as::<_, WidgetToken>(
        "SELECT * FROM widget_tokens
         WHERE project_id = ? AND mode = ? AND enabled = 1
           AND (expires_at IS NULL OR expires_at > datetime('now'))
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(mode)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())
}

/// 取该项目某模式下可用的接入 token：优先复用现有 enabled 的，否则新建一个。
/// mode="triage" 供反馈 widget；mode="flow" 供项目「需求入口」的 webhook 接入。
///
/// 并发安全：0075 的部分唯一索引保证每个 (project_id, mode) 至多一条 enabled，
/// 因此并发调用（React StrictMode 双挂载）中后到者的 INSERT 会被索引拒绝——
/// 此时回退再查一次，复用赢家插入的那条，绝不产生重复 token。
async fn ensure_token(
    db: &crate::db::Db,
    project_id: &str,
    mode: &str,
    label: &str,
) -> Result<WidgetToken, String> {
    if let Some(t) = fetch_enabled_token(db, project_id, mode).await? {
        return Ok(t);
    }
    match insert_widget_token(db, project_id, label, mode).await {
        Ok(t) => Ok(t),
        // INSERT 失败的最常见原因就是并发对手已抢先插入（唯一索引冲突）——回退复用它。
        Err(_) => fetch_enabled_token(db, project_id, mode)
            .await?
            .ok_or_else(|| "签发接入 token 失败".to_string()),
    }
}

/// 轮换某项目某模式的 token：在单事务内吊销现有所有 enabled 的同 mode token，再签发一条新的。
/// 事务保证「吊销→签发」原子，不会在中途暴露「零 enabled」窗口给并发的 ensure_token，
/// 也就不会撞 0075 唯一索引。旧 token 立即失效（用于泄露后重置 / 主动刷新）。
async fn rotate_token(
    db: &crate::db::Db,
    project_id: &str,
    mode: &str,
    label: &str,
) -> Result<WidgetToken, String> {
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE widget_tokens SET enabled = 0 WHERE project_id = ? AND mode = ? AND enabled = 1")
        .bind(project_id)
        .bind(mode)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
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
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let row = sqlx::query_as::<_, WidgetToken>("SELECT * FROM widget_tokens WHERE id = ?")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(row)
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

/// 为项目签发反馈 widget（triage）token：轮换式——吊销现有 enabled triage 再发新的，
/// 保证每个项目至多一条 enabled triage（与 0075 唯一索引一致）。
#[tauri::command]
pub async fn create_widget_token(
    project_id: String,
    label: Option<String>,
    state: State<'_, AppState>,
) -> Result<WidgetToken, String> {
    let label = label.unwrap_or_else(|| "默认".to_string());
    rotate_token(&state.db, &project_id, "triage", &label).await
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
    rotate_token(&state.db, &project_id, "flow", "需求入口").await
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
/// widget 与「需求入口」共用同一个项目级 flow token（与面板展示完全一致，消除"token 不对应"）；
/// 但 widget.js 提交时带 `?src=widget` 来源标记，webhook 端据此**强制落入待整理池**
/// （不自动烧 AI 预算）—— 见 intake/webhook.rs。来源标记只能把模式降级为 triage，
/// 不能提权为 flow，故公开嵌入此 token 不会让匿名访客绕过待整理直接进分析队列。
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
    let token = ensure_token(&state.db, &project_id, "flow", "需求入口").await?.token;
    let base = format!("http://localhost:{port}");
    Ok(format!(
        "<script src=\"{base}/widget.js\"\n        data-endpoint=\"{base}\"\n        data-project-id=\"{project_id}\"\n        data-api-key=\"{token}\"></script>"
    ))
}
