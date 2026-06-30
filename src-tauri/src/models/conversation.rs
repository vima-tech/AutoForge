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
    #[sqlx(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub from_agent: Option<String>,
    pub content_json: String,
    pub created_at: String,
    #[sqlx(default)]
    pub excluded_from_context: bool,
    /// 被引用/回复的消息 id（可空）。支持会议室「回复某条消息」的引用线索（迁移 0078）。
    #[sqlx(default)]
    pub parent_message_id: Option<String>,
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
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    pub conversation_id: String,
    pub content_json: String,
    /// 可选：本条消息引用/回复的目标消息 id（迁移 0078）。
    #[serde(default)]
    pub parent_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationAttachment {
    pub id: String,
    pub conversation_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub rel_path: String,
    pub mime: String,
    pub kind: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentUpload {
    pub conversation_id: String,
    pub file_name: String,
    pub mime_hint: String,
    pub data_base64: String,
}
