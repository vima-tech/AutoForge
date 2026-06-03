use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AdminDecision {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub change_request_id: Option<String>,
    pub stage: String,
    pub decision: String,
    pub admin_id: String,
    pub suggestions: Option<String>,
    pub created_at: String,
}
