use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TestSession {
    pub id: String,
    pub project_id: String,
    pub session_type: String,
    pub change_request_id: Option<String>,
    pub trigger: String,
    pub status: String,
    pub summary: String,
    pub results_json: String,
    pub issues_created: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ScanFinding {
    pub id: String,
    pub test_session_id: String,
    pub check_type: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub file_path: Option<String>,
    pub line_number: Option<i64>,
    pub fingerprint: String,
    pub issue_entry_id: Option<String>,
    pub created_at: String,
}
