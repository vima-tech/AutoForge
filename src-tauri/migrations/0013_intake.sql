-- Extend issues with external origin reference
ALTER TABLE issues ADD COLUMN source_ref TEXT;

-- Intake channel configuration (singleton row)
CREATE TABLE IF NOT EXISTS intake_configs (
    id                TEXT PRIMARY KEY DEFAULT 'singleton',
    webhook_enabled   INTEGER NOT NULL DEFAULT 0,
    webhook_port      INTEGER NOT NULL DEFAULT 27182,
    webhook_token     TEXT NOT NULL DEFAULT '',
    github_owner      TEXT NOT NULL DEFAULT '',
    github_repo       TEXT NOT NULL DEFAULT '',
    github_token      TEXT NOT NULL DEFAULT '',
    github_project_id TEXT NOT NULL DEFAULT '',
    github_last_sync  TEXT,
    ci_watch_dir      TEXT NOT NULL DEFAULT '',
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO intake_configs (id) VALUES ('singleton');

-- Track synced GitHub Issues to prevent re-import
CREATE TABLE IF NOT EXISTS github_synced_issues (
    github_number  INTEGER NOT NULL,
    repo_full_name TEXT NOT NULL,
    issue_id       TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    synced_at      TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (github_number, repo_full_name)
);
