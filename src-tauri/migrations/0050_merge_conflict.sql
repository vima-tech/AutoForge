-- 并发合并冲突处理：合并前自动把 dev merge 进 CR 分支，冲突时保留现场并进入
-- merge_conflict 态（区别于 merge_failed），供审核页三方视图 / 一键重试 / AI 解冲突使用。
-- app_settings 的开关键 auto_conflict_resolve_enabled 由 gate.rs 按需 INSERT，无需在此预置。

ALTER TABLE worktree_sessions ADD COLUMN conflict_files TEXT;  -- 冲突文件路径 JSON 数组
ALTER TABLE worktree_sessions ADD COLUMN conflict_diff TEXT;   -- 带冲突标记的快照（三方视图 / agent 输入）
