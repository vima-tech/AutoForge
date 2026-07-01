-- 会议室单条消息软删除：右键消息气泡删除时只标记 deleted_at，不物理删除。
-- 列表/未读/预览查询均过滤 deleted_at IS NULL；消息本体保留、可后续恢复或审计。
ALTER TABLE messages ADD COLUMN deleted_at TEXT;
CREATE INDEX IF NOT EXISTS ix_messages_deleted ON messages(conversation_id, deleted_at);
