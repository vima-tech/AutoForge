use serde::Serialize;

/// 一条 trace span 行（root agent / llm / tool / mcp 通用）。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct LlmTrace {
    pub id: String,
    pub trace_id: String,
    pub parent_id: Option<String>,
    pub seq: i64,
    pub kind: String,
    pub name: Option<String>,

    pub agent_id: Option<String>,
    pub agent_role: Option<String>,
    pub agent_name: Option<String>,
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub conversation_id: Option<String>,
    pub task_id: Option<String>,

    pub provider: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,

    pub status: String,
    pub error: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,

    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub latency_ms: Option<i64>,

    pub metadata_json: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub created_at: String,

    /// 派生字段（非存储列）：本次调用的 system_prompt 是否注入了 Innate 记忆召回小节。
    /// 由查询层用 `system_prompt LIKE` 计算（见 `commands/trace.rs`），供 Trace 页面打 INNATE 标志。
    pub innate_triggered: bool,
}

/// trace 列表项：每个 trace_id 的 root(agent) 汇总 + 统计（供列表页）。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct LlmTraceSummary {
    pub trace_id: String,
    pub agent_id: Option<String>,
    pub agent_role: Option<String>,
    pub agent_name: Option<String>,
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub conversation_id: Option<String>,
    pub task_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub total_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub span_count: i64,
    pub created_at: String,

    /// 派生字段（非存储列）：本 trace 的 root span 是否注入了 Innate 记忆召回（见 `commands/trace.rs`）。
    pub innate_triggered: bool,
}
