use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PrototypePrompt {
    pub id: String,
    pub project_id: String,
    pub issue_id: Option<String>,
    pub tool_target: String,
    pub title: String,
    pub prompt: String,
    pub created_at: String,
}
