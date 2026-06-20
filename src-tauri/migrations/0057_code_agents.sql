-- 代码 Agent 可插拔：可选的代码实现 agent 配置（claude / codex / opencode 互换）。
-- 见 .autoforge/docs/待实现功能/AutoForge-代码Agent可插拔-功能提案.md
CREATE TABLE IF NOT EXISTS code_agents (
  id              TEXT PRIMARY KEY,
  kind            TEXT NOT NULL,                       -- 'claude' | 'codex' | 'opencode'
  label           TEXT NOT NULL,
  program         TEXT NOT NULL,                       -- 可执行名/绝对路径
  model           TEXT,                                -- 可空，映射各家 --model/-m
  extra_args_json TEXT NOT NULL DEFAULT '[]',          -- 追加 flag（JSON 字符串数组）
  enabled         INTEGER NOT NULL DEFAULT 1,
  is_default      INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 种子三条内置 agent；claude 为默认。
INSERT OR IGNORE INTO code_agents (id, kind, label, program, model, enabled, is_default) VALUES
  ('claude',   'claude',   'Claude Code', 'claude',   NULL, 1, 1),
  ('codex',    'codex',    'Codex CLI',   'codex',    NULL, 1, 0),
  ('opencode', 'opencode', 'OpenCode',    'opencode', NULL, 1, 0);

-- 项目级覆盖：NULL = 跟随全局默认。
ALTER TABLE projects ADD COLUMN code_agent_id TEXT;
