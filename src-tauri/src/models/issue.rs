use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Issue {
    pub id: String,
    pub project_id: String,
    pub source_type: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub severity: String,
    pub priority: Option<i64>,
    pub status: String,
    pub fingerprint: String,
    #[sqlx(default)]
    pub source_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IssueAnalysis {
    pub id: String,
    pub issue_id: String,
    pub authenticity_score: f64,
    pub feasibility_score: Option<f64>,
    pub priority_suggestion: Option<i64>,
    pub category_suggestion: Option<String>,
    pub severity_suggestion: Option<String>,
    pub duplicate_of: Option<String>,
    pub affected_modules: Option<String>,
    pub analysis_summary: String,
    pub raw_llm_output: Option<String>,
    #[sqlx(default)]
    pub analysis_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIssue {
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub source_type: Option<String>,
    pub source_ref: Option<String>,
}
