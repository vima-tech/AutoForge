-- 会议室消息引用回复（#13）：messages 增加 parent_message_id，指向被引用/回复的消息。
-- 可空外键（普通消息不填，行为不变）；用于「回复某条消息」的线索与前端引用渲染。
ALTER TABLE messages ADD COLUMN parent_message_id TEXT REFERENCES messages(id);
CREATE INDEX IF NOT EXISTS ix_messages_parent ON messages(parent_message_id);
