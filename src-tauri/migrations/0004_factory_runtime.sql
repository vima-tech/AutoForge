CREATE TABLE IF NOT EXISTS preview_environments (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(id),
    env_type            TEXT NOT NULL DEFAULT 'worktree',
    worktree_session_id TEXT REFERENCES worktree_sessions(id),
    container_id        TEXT,
    preview_url         TEXT NOT NULL DEFAULT '',
    db_snapshot_name    TEXT,
    status              TEXT NOT NULL DEFAULT 'pending',
    data_masked_at      TEXT,
    mask_policy_version TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    ready_at            TEXT,
    terminated_at       TEXT
);
CREATE INDEX IF NOT EXISTS ix_preview_project ON preview_environments(project_id);
CREATE INDEX IF NOT EXISTS ix_preview_session ON preview_environments(worktree_session_id);
CREATE INDEX IF NOT EXISTS ix_preview_status  ON preview_environments(status);

CREATE TABLE IF NOT EXISTS test_sessions (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    session_type      TEXT NOT NULL DEFAULT 'reactive',
    change_request_id TEXT REFERENCES change_requests(id),
    trigger           TEXT NOT NULL DEFAULT 'merge',
    status            TEXT NOT NULL DEFAULT 'pending',
    summary           TEXT NOT NULL DEFAULT '',
    results_json      TEXT NOT NULL DEFAULT '{}',
    issues_created    TEXT NOT NULL DEFAULT '[]',
    started_at        TEXT,
    completed_at      TEXT
);
CREATE INDEX IF NOT EXISTS ix_test_project ON test_sessions(project_id);
CREATE INDEX IF NOT EXISTS ix_test_cr      ON test_sessions(change_request_id);
CREATE INDEX IF NOT EXISTS ix_test_status  ON test_sessions(status);

CREATE TABLE IF NOT EXISTS scan_findings (
    id              TEXT PRIMARY KEY,
    test_session_id TEXT NOT NULL REFERENCES test_sessions(id),
    check_type      TEXT NOT NULL DEFAULT 'test',
    severity        TEXT NOT NULL DEFAULT 'medium',
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    file_path       TEXT,
    line_number     INTEGER,
    fingerprint     TEXT NOT NULL DEFAULT '',
    issue_entry_id  TEXT REFERENCES issues(id),
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_findings_session ON scan_findings(test_session_id);
CREATE INDEX IF NOT EXISTS ix_findings_issue   ON scan_findings(issue_entry_id);

CREATE TABLE IF NOT EXISTS admin_decisions (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    issue_id          TEXT NOT NULL REFERENCES issues(id),
    change_request_id TEXT REFERENCES change_requests(id),
    stage             TEXT NOT NULL,
    decision          TEXT NOT NULL,
    admin_id          TEXT NOT NULL DEFAULT 'admin',
    suggestions       TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_decisions_project ON admin_decisions(project_id);
CREATE INDEX IF NOT EXISTS ix_decisions_issue   ON admin_decisions(issue_id);
CREATE INDEX IF NOT EXISTS ix_decisions_cr      ON admin_decisions(change_request_id);
