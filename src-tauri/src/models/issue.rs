use serde::{Deserialize, Serialize};

/// 需求列表的统一「严重度优先」排序片段（SQL 片段，紧跟在 `WHERE …` 之后拼接）。
///
/// 修复「优先级倒挂」：历史上所有列表一律 `ORDER BY created_at`，于是一条 `critical`
/// 缺陷会被当天新建的一堆 `medium` 功能压在最底，淹没在噪音里。改为**先按严重度降序**，
/// 同级再按时间——让 critical/high 永远浮在最前。各调用点在此片段后追加 `created_at`
/// 方向与 `LIMIT/OFFSET`。
///
/// 注意末尾的逗号：使用方紧跟 `created_at ASC|DESC ...`。
pub const SEVERITY_ORDER_PREFIX: &str =
    " ORDER BY (CASE severity WHEN 'critical' THEN 3 WHEN 'high' THEN 2 WHEN 'medium' THEN 1 ELSE 0 END) DESC, ";

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
    /// 需求审核 管理员补充意见（一次性）：提交后需求带此意见重新分析，被分析任务消费后清空。
    #[sqlx(default)]
    pub review_feedback: Option<String>,
    /// 该需求是否由「恢复需求」从已撤销态重回队列（1=是）。用于队列里的小 dot 标识。
    #[sqlx(default)]
    pub restored_from_revert: i64,
    /// 会议室「立即编码」幂等键（前端 UUID），唯一索引防重复创建。
    #[sqlx(default)]
    pub client_request_id: Option<String>,
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
