-- 代码审核通过时人工填写的合并提交信息（merge --no-ff -m <msg>）。
-- 空/NULL 时合并任务回退默认模板 "AutoForge merge: <cr_id>"。
-- 持久化到 CR 行而非 job payload：retry_merge 与 AI 解冲突回落合并复用同一条。
ALTER TABLE change_requests ADD COLUMN merge_commit_message TEXT;
