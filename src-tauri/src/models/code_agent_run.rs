//! 代码 Agent 单次执行的完整日志（见迁移 0064）。
//! `CodeAgentRunLog` 含完整 stdout/stderr，供详情查看；`CodeAgentRunMeta` 不含正文，
//! 用于列表（避免一次拉回多条大文本）。

use serde::Serialize;

/// 完整一条运行日志（含 stdout/stderr 正文）。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct CodeAgentRunLog {
    pub id: String,
    pub change_request_id: String,
    pub worktree_session_id: Option<String>,
    pub phase: String,
    pub kind: String,
    pub model: Option<String>,
    pub exit_code: i64,
    pub duration_ms: i64,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: i64,
    pub stderr_bytes: i64,
    pub truncated: i64,
    pub created_at: String,
}

/// 列表用的轻量元信息（不含正文，只带字节数）。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct CodeAgentRunMeta {
    pub id: String,
    pub change_request_id: String,
    pub worktree_session_id: Option<String>,
    pub phase: String,
    pub kind: String,
    pub model: Option<String>,
    pub exit_code: i64,
    pub duration_ms: i64,
    pub stdout_bytes: i64,
    pub stderr_bytes: i64,
    pub truncated: i64,
    pub created_at: String,
}
