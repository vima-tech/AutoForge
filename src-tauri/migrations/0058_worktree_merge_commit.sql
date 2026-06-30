-- 记录 CR 合并到 dev 时产生的提交 SHA，作为「稳定撤销」的对象。
-- 合并改用 `git merge --squash` + `commit` 后，每个 CR 在 dev 上是【一个普通提交】，
-- 撤销即 `git revert <sha>`（普通提交无需 -m 1）。此前从不记录该 SHA，撤销无从下手。
-- 合并成功后 `git rev-parse HEAD` 回填此列；旧 session 为 NULL → 前端撤销入口置灰降级。
ALTER TABLE worktree_sessions ADD COLUMN merge_commit TEXT;
