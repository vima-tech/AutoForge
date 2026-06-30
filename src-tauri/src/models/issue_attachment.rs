use serde::{Deserialize, Serialize};

/// 需求条目附件记录。镜像 [`crate::models::conversation::ConversationAttachment`]，
/// 仅外键挂在 issue 上。`kind` 为 image/file（白名单判定见 attachments_common），
/// `rel_path` 形如 `<issue_id>/<uuid>.<ext>`，相对 `attachments_base()/issues/`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IssueAttachment {
    pub id: String,
    pub issue_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub rel_path: String,
    pub mime: String,
    pub kind: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at: String,
}

/// 前端上传需求附件的载荷：base64 内容 + 文件名 + MIME 提示。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAttachmentUpload {
    pub issue_id: String,
    pub file_name: String,
    pub mime_hint: String,
    pub data_base64: String,
}
