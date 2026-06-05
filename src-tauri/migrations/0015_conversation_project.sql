-- Bind group conversations to a project
ALTER TABLE conversations ADD COLUMN project_id TEXT REFERENCES projects(id);

-- Track which project files are pinned as on-demand context for a conversation
CREATE TABLE IF NOT EXISTS conversation_project_context (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    rel_path        TEXT NOT NULL,
    added_at        TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (conversation_id, rel_path)
);
