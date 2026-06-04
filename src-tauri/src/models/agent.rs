use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub name_en: String,
    pub role: String,
    pub color: String,
    pub initial: String,
    pub llm_id: Option<String>,
    pub system_prompt: String,
    pub forge_role: Option<String>,
    pub role_type: String,
    pub system_kind: Option<String>,
    pub capabilities_json: String,
    pub max_concurrency: i64,
    pub visible_in_chat: bool,
    pub mentionable: bool,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgent {
    pub name: String,
    pub name_en: Option<String>,
    pub role: Option<String>,
    pub color: Option<String>,
    pub initial: Option<String>,
    pub llm_id: Option<String>,
    pub system_prompt: Option<String>,
    pub role_type: Option<String>,
    pub system_kind: Option<String>,
    pub capabilities_json: Option<String>,
    pub max_concurrency: Option<i64>,
    pub visible_in_chat: Option<bool>,
    pub mentionable: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub name_en: Option<String>,
    pub role: Option<String>,
    pub color: Option<String>,
    pub llm_id: Option<Option<String>>,
    pub system_prompt: Option<String>,
    pub forge_role: Option<Option<String>>,
    pub role_type: Option<String>,
    pub system_kind: Option<Option<String>>,
    pub capabilities_json: Option<String>,
    pub max_concurrency: Option<i64>,
    pub visible_in_chat: Option<bool>,
    pub mentionable: Option<bool>,
    pub enabled: Option<bool>,
}
