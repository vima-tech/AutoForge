-- 代码 Agent（claude / codex / opencode）单次执行的完整日志，便于发现/调试问题。
-- 不同于 worktree_sessions.report_content（仅存抽取后的报告或失败尾部）：本表保留
-- 该次进程的完整 stdout/stderr（含超时被杀前的输出），按 created_at 滚动保留。
CREATE TABLE IF NOT EXISTS code_agent_run_logs (
  id                   TEXT PRIMARY KEY,
  change_request_id    TEXT NOT NULL,
  worktree_session_id  TEXT,
  -- 执行阶段：execution（代码实现）/ conflict_resolve（AI 解合并冲突）。
  phase                TEXT NOT NULL DEFAULT 'execution',
  kind                 TEXT NOT NULL,          -- claude / codex / opencode
  model                TEXT,
  exit_code            INTEGER NOT NULL,
  duration_ms          INTEGER NOT NULL DEFAULT 0,
  stdout               TEXT NOT NULL DEFAULT '',
  stderr               TEXT NOT NULL DEFAULT '',
  -- 原始字节数（截断前），用于在前端提示"已截断"。
  stdout_bytes         INTEGER NOT NULL DEFAULT 0,
  stderr_bytes         INTEGER NOT NULL DEFAULT 0,
  truncated            INTEGER NOT NULL DEFAULT 0,
  created_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_car_logs_cr ON code_agent_run_logs(change_request_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_car_logs_created ON code_agent_run_logs(created_at);
