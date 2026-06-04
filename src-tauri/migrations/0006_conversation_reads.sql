CREATE TABLE IF NOT EXISTS conversation_reads (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
    read_at         TEXT NOT NULL DEFAULT (datetime('now'))
);
