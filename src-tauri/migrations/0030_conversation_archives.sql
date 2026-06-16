-- 会议室对话归档：把会议室当前的消息打包成不可变的只读快照，
-- 便于后续检索与回顾。归档后原会议室消息被清空（房间保留可继续使用）。
-- payload_json 保存渲染所需的去规范化消息数组；search_text 为全文检索用的纯文本。
CREATE TABLE IF NOT EXISTS conversation_archives (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    conv_type       TEXT NOT NULL,
    title           TEXT NOT NULL,
    project_id      TEXT,
    project_name    TEXT,
    message_count   INTEGER NOT NULL DEFAULT 0,
    payload_json    TEXT NOT NULL,
    search_text     TEXT NOT NULL DEFAULT '',
    archived_at     TEXT NOT NULL DEFAULT (datetime('now')),
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_conv_archives_time ON conversation_archives(archived_at DESC);
CREATE INDEX IF NOT EXISTS idx_conv_archives_conv ON conversation_archives(conversation_id);
