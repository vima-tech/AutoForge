-- 角色 Agent 提示词模式：builtin（用内置专业提示词）/ append（内置 + 补充）/ custom（完全自定义）。
-- 默认 'builtin'：系统角色与有内置提示词的角色直接采用专业基线；
-- 没有内置提示词的自定义对话角色，在提示词拼接时自动退回其自身 system_prompt（等价 custom）。
ALTER TABLE agents ADD COLUMN prompt_mode TEXT NOT NULL DEFAULT 'builtin';
