-- 固定 CR diff 的对比基线：worktree 从 dev 分叉时记录当时 dev 的提交 SHA。
-- 此前 get_code_diff/merge 快照都用 `git diff <dev>`（活动 HEAD）作对比，
-- 一旦 dev 在 CR 执行后独立前进（如并行合并了别的批次），实时 diff 会把
-- 那些无关改动反向泄漏进本 CR 的 diff，导致审核页 diff 失真。
-- 改为对这个不可变的分叉点 SHA 做 diff，dev 怎么移动都不影响 CR 的真实改动。
-- 旧 session 该列为 NULL，diff 计算回退到原 target_branch 行为，向后兼容。

ALTER TABLE worktree_sessions ADD COLUMN base_commit TEXT;
