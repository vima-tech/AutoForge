use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Conversation {
    pub id: String,
    #[sqlx(rename = "type")]
    pub conv_type: String,
    pub name: Option<String>,
    pub color: String,
    pub initial: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub from_agent: Option<String>,
    pub content_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationDetail {
    pub id: String,
    pub conv_type: String,
    pub name: Option<String>,
    pub color: String,
    pub initial: Option<String>,
    pub created_at: String,
    pub members: Vec<String>,
    pub unread: i64,
    pub last_message: Option<String>,
    pub last_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    pub conversation_id: String,
    pub content_json: String,
}
