use serde::{Deserialize, Serialize};

/// 归档快照中的单条消息（去规范化，便于离开 live agents 也能原样只读渲染）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedMessage {
    pub from_agent: Option<String>,
    /// 归档时刻的发言人显示名（"我" / Innate / Agent 名）。
    pub author: String,
    /// Agent 英文名（群聊作者 chip 用），无则空串。
    pub author_en: String,
    /// Agent 主题色，无则空串。
    pub author_color: String,
    pub is_me: bool,
    pub is_innate: bool,
    pub content_json: String,
    pub created_at: String,
    pub excluded_from_context: bool,
}

/// 归档记录摘要（列表 / 搜索结果用，不含完整 payload，体积小）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConversationArchiveSummary {
    pub id: String,
    pub conversation_id: String,
    pub conv_type: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub message_count: i64,
    pub archived_at: String,
}

/// 归档详情（回顾时拉取完整消息快照）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConversationArchiveDetail {
    pub id: String,
    pub conversation_id: String,
    pub conv_type: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub message_count: i64,
    pub payload_json: String,
    pub archived_at: String,
}

/// 全文搜索命中（摘要 + 命中次数 + 片段）。
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveSearchHit {
    #[serde(flatten)]
    pub summary: ConversationArchiveSummary,
    pub match_count: i64,
    pub snippet: String,
}
