use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PreviewEnvironment {
    pub id: String,
    pub project_id: String,
    pub env_type: String,
    pub worktree_session_id: Option<String>,
    pub container_id: Option<String>,
    pub preview_url: String,
    pub db_snapshot_name: Option<String>,
    pub status: String,
    pub data_masked_at: Option<String>,
    pub mask_policy_version: Option<String>,
    pub created_at: String,
    pub ready_at: Option<String>,
    pub terminated_at: Option<String>,
}
