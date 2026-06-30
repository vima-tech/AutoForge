-- 合并测试门并行化：把 merge 拆成 premerge(并行,无 merge_lock) + land(串行,持锁)。
-- premerge 记录测试所基于的 dev 提交，land 阶段据此「再校验」dev 是否在测后落地前前进。
ALTER TABLE worktree_sessions ADD COLUMN tested_dev_sha TEXT;
ALTER TABLE worktree_sessions ADD COLUMN premerge_at TEXT;
