use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotifyChannel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub events: String,
    pub enabled: bool,
    pub created_at: String,
}
