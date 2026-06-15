-- Node 08: deployment records (design "08 部署上线")
CREATE TABLE IF NOT EXISTS deployments (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    change_request_id TEXT,
    target_env        TEXT NOT NULL DEFAULT 'production',
    script            TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'pending_confirm', -- pending_confirm | running | deployed | failed
    log               TEXT NOT NULL DEFAULT '',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    confirmed_at      TEXT,
    completed_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_deployments_project ON deployments(project_id);
