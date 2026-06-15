use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct JobExecution {
    pub id: String,
    pub idempotency_key: String,
    pub job_type: String,
    pub payload: String,
    pub status: String,
    pub attempt: i64,
    pub last_error: Option<String>,
    pub enqueued_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum JobPayload {
    Analysis {
        issue_id: String,
    },
    Execution {
        change_request_id: String,
        project_id: String,
    },
    Testing {
        change_request_id: String,
    },
    Merge {
        change_request_id: String,
    },
    SecurityAudit {
        change_request_id: String,
    },
}
