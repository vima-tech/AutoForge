-- 孵化台深化（孵化台深化/设计方案 §5、§6.1）：为「带控制通道的 Agent 起草会话」预留数据位。
-- 纯新增列，不改既有语义（迁移只增不改）。内部命名继续留 blueprint_*（避免无谓 churn）。
--
--   eval_json        —— 每次落稿后 critic 的四维打分 + 待补强项（P3 默认多轮评估）
--   pending_question —— ask_user 挂起时的待答问题文本（+可选 options）（P2 断点续跑）
--   context_json     —— 每稿的上下文账本：pinned 文件 / codegraph 命中 / 已答决策 /
--                       工具产出摘要 / eval 结论。结构对齐基质 ContextItem 数组（§4 兜底），
--                       便于日后零改造收编进 context_index。
ALTER TABLE blueprint_drafts ADD COLUMN eval_json        TEXT NOT NULL DEFAULT '';
ALTER TABLE blueprint_drafts ADD COLUMN pending_question TEXT NOT NULL DEFAULT '';
ALTER TABLE blueprint_drafts ADD COLUMN context_json     TEXT NOT NULL DEFAULT '';

-- status 值域新增 'awaiting_answer'（待答复派生态）——自由字符串列，无需 DDL。
-- blueprint_messages.role 值域新增 'question'/'answer'/'eval'/'tool'——自由字符串，无需 DDL。
