use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IntakeConfig {
    pub id: String,
    pub webhook_enabled: bool,
    pub webhook_port: i64,
    pub webhook_token: String,
    pub github_owner: String,
    pub github_repo: String,
    pub github_token: String,
    pub github_project_id: String,
    pub github_last_sync: Option<String>,
    pub ci_watch_dir: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateIntakeConfig {
    pub webhook_enabled: Option<bool>,
    pub webhook_port: Option<i64>,
    pub webhook_token: Option<String>,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub github_token: Option<String>,
    pub github_project_id: Option<String>,
    pub ci_watch_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub errors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub found: u32,
    pub new_issues: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResult {
    pub total: u32,
    pub imported: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookStatus {
    pub running: bool,
    pub port: u16,
}
