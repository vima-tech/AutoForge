-- 会议室对话软删除：右键删除对话时只标记 deleted_at，不物理删除记录与历史消息。
-- 列表查询过滤 deleted_at IS NULL；软删除的对话对用户隐藏但数据保留、可后续恢复/审计。
ALTER TABLE conversations ADD COLUMN deleted_at TEXT;
CREATE INDEX IF NOT EXISTS ix_conversations_deleted ON conversations(deleted_at);
