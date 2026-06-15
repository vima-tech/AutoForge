use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub change_request_id: Option<String>,
    pub target_env: String,
    pub script: String,
    pub status: String,
    pub log: String,
    pub created_at: String,
    pub confirmed_at: Option<String>,
    pub completed_at: Option<String>,
}
