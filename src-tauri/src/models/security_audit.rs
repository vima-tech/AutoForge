use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SecurityAudit {
    pub id: String,
    pub project_id: String,
    pub change_request_id: Option<String>,
    pub status: String,
    pub severity: String,
    pub summary: String,
    pub findings_json: String,
    pub issues_created: String,
    pub started_at: String,
    pub completed_at: Option<String>,
}
