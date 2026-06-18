use serde::{Deserialize, Serialize};

/// M10 反馈 widget 专用接入 token（项目级、可独立吊销）。
/// 与 `intake_configs.webhook_token` 解耦，详见 migration `0041_widget_tokens.sql`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WidgetToken {
    pub id: String,
    pub project_id: String,
    pub token: String,
    pub label: String,
    pub enabled: bool,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
}
