use serde::{Deserialize, Serialize};

/// 项目级需求接入 token（可独立吊销）。统一承载两类入站凭证：
///   * `mode = "flow"`   —— 项目「需求入口」签发的 webhook 接入 token，自动进分析队列；
///   * `mode = "triage"` —— 匿名反馈 widget token，进待整理池，不自动烧 AI 预算。
/// 入站请求的项目由本表 token 单向推导（webhook.rs），不再信任 payload.project_id。
/// 详见 migration `0041_widget_tokens.sql` 与 `0053_intake_token_mode.sql`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WidgetToken {
    pub id: String,
    pub project_id: String,
    pub token: String,
    pub label: String,
    pub enabled: bool,
    /// "flow" | "triage"，决定经此 token 投递的需求落库模式。
    pub mode: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
}
