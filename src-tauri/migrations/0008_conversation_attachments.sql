CREATE TABLE IF NOT EXISTS conversation_attachments (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    original_name   TEXT NOT NULL,
    stored_name     TEXT NOT NULL,
    rel_path        TEXT NOT NULL UNIQUE,
    mime            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    sha256          TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_conversation_attachments_conv
    ON conversation_attachments(conversation_id, created_at);
