-- 会议室「立即编码」express 路径：CR 携带会议室讨论上下文（对话快照 + 项目上下文文档），
-- 在 build_prompt 里注入编码工单，作为「需求来源」段。普通流水线 CR 该列为 NULL，行为不变。
ALTER TABLE change_requests ADD COLUMN work_context TEXT;
