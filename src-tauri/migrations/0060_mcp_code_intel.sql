-- 统一 MCP code-intel 接入：把「执行前代码情报预查」从硬编码 codegraph 改为可配置的
-- MCP 提供者。复用现有 mcp_servers 表 + MCP client，换工具只改配置、零 Rust 改动。
--   role='code_intel'    → 该 server 是代码情报提供者（由 AutoForge 在执行前 push 式调用）。
--   capability_map_json  → 能力→工具映射，参数支持 $SYMBOL / $REPO 占位符。
-- 与 agents.capabilities_json（Agent 工具白名单）无关，故另起列名避免混淆。
ALTER TABLE mcp_servers ADD COLUMN role                TEXT NOT NULL DEFAULT '';
ALTER TABLE mcp_servers ADD COLUMN capability_map_json TEXT NOT NULL DEFAULT '{}';

-- 种子 codegraph 为默认 code_intel 提供者（stdio: `codegraph serve --mcp`）。
-- agent_ids_json='[]' → 不作为 Agent 工具加载（push-only，不暴露给 worktree 内 agent）。
-- 未装 codegraph 时连接失败 → 预查优雅退化为空，不阻断流水线。
INSERT OR IGNORE INTO mcp_servers
  (id, name, transport, command, args_json, role, capability_map_json, agent_ids_json, enabled)
VALUES (
  'code-intel-codegraph', 'codegraph', 'stdio', 'codegraph', '["serve","--mcp"]', 'code_intel',
  '{"locate_symbol":{"tool":"codegraph_search","args":{"query":"$SYMBOL","projectPath":"$REPO","limit":1}},"find_callers":{"tool":"codegraph_callers","args":{"symbol":"$SYMBOL","projectPath":"$REPO","limit":5}}}',
  '[]', 1
);
