-- 原型提示词必须对应一个孵化台需求（draft）。
-- 历史遗留的、未关联任何需求（draft_id 为空/NULL）的旧原型提示词一次性清理删除，
-- 使 prototype_prompts 中每一条都归属某个 blueprint_draft。此后生成入口在后端硬拒空 draft_id。
DELETE FROM prototype_prompts WHERE draft_id IS NULL OR TRIM(draft_id) = '';
