-- 精简 MCP 适用面模型：去掉 role 互斥分类（普通工具源 / 代码情报），改为独立开关。
-- 一个 server 的适用面由两个正交维度决定，可同时成立：
--   agent_ids_json   → 适用于哪些「角色 Agent」（会议室，pull 原始工具）
--   for_code_agent   → 是否适用于「编码 Agent」（执行前 push，按 capability_map 预查注入）
-- capability_map_json 是 server 属性，不再绑定某个 role；仅 push 流（编码 Agent）会用到它。
ALTER TABLE mcp_servers ADD COLUMN for_code_agent INTEGER NOT NULL DEFAULT 0;

-- 把旧 role='code_intel' 迁移为 for_code_agent=1。role 列保留但不再使用（SQLite 不便删列）。
UPDATE mcp_servers SET for_code_agent = 1 WHERE role = 'code_intel';
