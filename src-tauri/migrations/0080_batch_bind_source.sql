-- 需求批量绑定（工单组）：记录每条成员关联的「绑定来源」，用于审计与相关度告警留痕。
--   'auto'   = compute_candidates 规则探测推荐后采纳（文件重叠）
--   'manual' = 人工圈选批量绑定（可能无文件重叠，经相关度二次确认）
-- 既有行皆为历史合并 / 单需求 CR primary，DEFAULT 'auto' 已覆盖，无需回填 UPDATE。
ALTER TABLE change_request_issues ADD COLUMN bind_source TEXT NOT NULL DEFAULT 'auto';
