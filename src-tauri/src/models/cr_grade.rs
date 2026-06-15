use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrGrade {
    pub change_request_id: String,
    pub tier: String,
    pub score: i64,
    pub rationale: String,
    pub change_class: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AutoPassPolicy {
    pub change_class: String,
    pub trust_state: String,
    pub approve_count: i64,
    pub reject_count: i64,
    pub updated_at: String,
}
