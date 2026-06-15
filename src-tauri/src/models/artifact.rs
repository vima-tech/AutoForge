use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeliveryArtifact {
    pub id: String,
    pub project_id: String,
    pub node: String,
    pub original_name: String,
    pub stored_name: String,
    pub rel_path: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub description: String,
    pub created_at: String,
}
