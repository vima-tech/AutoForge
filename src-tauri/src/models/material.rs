use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MaterialFolder {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MaterialFile {
    pub id: String,
    pub project_id: String,
    pub folder_id: Option<String>,
    pub original_name: String,
    pub stored_name: String,
    pub rel_path: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub description: Option<String>,
    pub tags: String,
    pub backup_status: String,
    pub backup_url: Option<String>,
    pub backed_up_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MaterialBackupConfig {
    pub id: String,
    pub provider: String,
    pub config_json: String,
    pub enabled: bool,
    pub updated_at: String,
}
