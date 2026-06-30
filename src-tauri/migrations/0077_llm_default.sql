-- 全局默认 LLM：未显式绑定 LLM 的角色 Agent 回落到该配置（治本：漏配不再致命）。
-- 复用 code_agents.is_default 同款单选范式（命令层保证至多一个 is_default=1）。
ALTER TABLE llm_configs ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
