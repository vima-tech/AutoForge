use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PrototypePrompt {
    pub id: String,
    pub project_id: String,
    pub issue_id: Option<String>,
    pub tool_target: String,
    pub title: String,
    pub prompt: String,
    /// 绑定的孵化台大需求草稿（按需求归档/过滤；空=项目级旧数据）。
    #[sqlx(default)]
    pub draft_id: String,
    /// 'new'（新页面）/ 'existing'（改现有页面）/ ''（未标注）。
    #[sqlx(default)]
    pub design_mode: String,
    pub created_at: String,
}
