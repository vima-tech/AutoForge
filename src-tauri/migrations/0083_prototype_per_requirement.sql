-- 原型设计从「全项目一个」升级为「按需求（孵化台草稿）」的实用功能：
--   draft_id     —— 绑定生成该原型的孵化台大需求草稿（一键从孵化台跳入 + 按需求过滤展示）
--   design_mode  —— 'new'（新页面）/ 'existing'（在现有页面基础上改动），供 UI 标注与追溯
-- 纯新增列，不改既有语义（旧数据 draft_id/design_mode 为空，行为=项目级原型，向后兼容）。
ALTER TABLE prototype_prompts ADD COLUMN draft_id    TEXT NOT NULL DEFAULT '';
ALTER TABLE prototype_prompts ADD COLUMN design_mode TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_prototype_prompts_draft ON prototype_prompts(draft_id);
