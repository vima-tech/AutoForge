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
    #[sqlx(default)]
    pub raw_capture: Option<String>,
    #[sqlx(default)]
    pub repro_steps: Option<String>,
    #[sqlx(default)]
    pub environment: Option<String>,
    #[sqlx(default)]
    pub expected: Option<String>,
    #[sqlx(default)]
    pub actual: Option<String>,
    #[sqlx(default)]
    pub acceptance_json: Option<String>,
    /// 审核 1 管理员补充意见（一次性）：提交后需求带此意见重新分析，被分析任务消费后清空。
    #[sqlx(default)]
    pub review_feedback: Option<String>,
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
    /// 入口模式："triage" 进待整理池不自动分析；其余/缺省走正常流水线。
    #[serde(default)]
    pub mode: Option<String>,
    /// Bug 载体字段（category='Bug' 时由前端展开填写），用于喂自主修复。
    #[serde(default)]
    pub repro_steps: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub actual: Option<String>,
}
