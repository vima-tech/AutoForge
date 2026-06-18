-- 持久化 CR 代码 diff：合并后 worktree 会被删除，实时 `git diff` 取不到改动，
-- 导致「已合并」需求在审核页看不到代码 diff。合并前把 diff 快照落库到此列，
-- get_code_diff 在 worktree 不存在时回退读取，保证历史改动永久可查。

ALTER TABLE worktree_sessions ADD COLUMN diff_content TEXT;
