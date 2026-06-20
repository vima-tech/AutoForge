-- 通知按「需求线程」折叠：同一条需求的各阶段事件（录入→分析→审核1→测试→审核2→合并→安全审计）
-- 共享一个 thread_key，写入时按该键 UPSERT 就地刷新，而非每个阶段新插一行，
-- 避免单条需求把收件箱刷屏（一条需求只占一行，内容随阶段推进而更新）。
ALTER TABLE notifications ADD COLUMN thread_key TEXT;

-- 唯一索引作为 UPSERT 的冲突目标。SQLite 视多个 NULL 互不相同，
-- 故历史无线程键的旧行（thread_key IS NULL）不受约束、彼此不冲突。
CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_thread ON notifications(thread_key);
