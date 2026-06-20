use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChangeRequest {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub status: String,
    pub admin_id: Option<String>,
    pub approved_at: Option<String>,
    pub admin_suggestions_1: Option<String>,
    pub admin_suggestions_2: Option<String>,
    pub merge_commit_message: Option<String>,
    pub target_branch: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review1Decision {
    pub decision: String,
    pub suggestions: Option<String>,
    pub admin_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review2Decision {
    pub decision: String,
    pub suggestions: Option<String>,
    pub admin_id: Option<String>,
    /// 人工填写的合并提交信息；空时合并任务回退默认模板。
    pub commit_message: Option<String>,
}
