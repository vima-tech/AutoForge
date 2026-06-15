-- 角色 Agent 是否启用 Innate 记忆召回（默认开启：让角色随经验越用越好）。
-- 仅对走 run_system_role_text 的系统角色生效；关闭则跳过召回。
ALTER TABLE agents ADD COLUMN memory_enabled INTEGER NOT NULL DEFAULT 1;
