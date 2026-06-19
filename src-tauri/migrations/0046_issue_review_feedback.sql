-- 审核 1 补充意见重评：管理员在「待需求审核」阶段可提交补充意见，
-- 让需求带着该意见重新分析，再回到审核 1。意见为一次性输入，
-- 被分析任务消费后清空（见 tasks/analysis.rs）。
ALTER TABLE issues ADD COLUMN review_feedback TEXT;
