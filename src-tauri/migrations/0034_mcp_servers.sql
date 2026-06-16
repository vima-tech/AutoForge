-- MCP server 配置。每个 server 通过 agent_ids_json 勾选其适用的 Agent（适用范围由
-- MCP 配置侧选择，而非每个 Agent 单独勾工具）。MVP 仅只读/无副作用工具，写类默认禁用。
--   transport=stdio → 本地子进程：command + args_json(数组) + env_json(对象)
--   transport=http  → 远程 streamable-http：url + headers_json(对象，含鉴权头)
CREATE TABLE IF NOT EXISTS mcp_servers (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    transport      TEXT NOT NULL DEFAULT 'stdio',   -- stdio | http
    command        TEXT NOT NULL DEFAULT '',
    args_json      TEXT NOT NULL DEFAULT '[]',
    env_json       TEXT NOT NULL DEFAULT '{}',
    url            TEXT NOT NULL DEFAULT '',
    headers_json   TEXT NOT NULL DEFAULT '{}',
    agent_ids_json TEXT NOT NULL DEFAULT '[]',       -- 适用 Agent id 列表
    enabled        INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
