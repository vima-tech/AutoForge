-- 需求分析完整结构化结果（issue_analysis.schema.json v1.0）
-- triage 字段仍保留独立列以兼容旧逻辑；此列存放整份 JSON，供 code_agent 拼装 Claude Code 工单。
ALTER TABLE issue_analyses ADD COLUMN analysis_json TEXT;
