use serde::Serialize;

/// 一条环节 Agent 的结构化产出（schema 驱动 agent 的统一落点）。
/// 完整行（含 raw）供详情；列表用 [`AgentOutputSummary`] 省去大字段。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct AgentOutput {
    pub id: String,
    pub role: String,
    pub schema_version: String,
    pub target_kind: String,
    pub target_id: String,
    pub project_id: Option<String>,
    pub trace_id: Option<String>,
    pub status: String,
    pub output_json: Option<String>,
    pub raw: Option<String>,
    pub created_at: String,
}

/// 列表项：省去 `raw`（审计原文），其余保留以便前端直接渲染结构化产出。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct AgentOutputSummary {
    pub id: String,
    pub role: String,
    pub schema_version: String,
    pub target_kind: String,
    pub target_id: String,
    pub project_id: Option<String>,
    pub trace_id: Option<String>,
    pub status: String,
    pub output_json: Option<String>,
    pub created_at: String,
}
