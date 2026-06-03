CREATE TABLE IF NOT EXISTS llm_configs (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    provider    TEXT NOT NULL DEFAULT 'Anthropic',
    model       TEXT NOT NULL,
    endpoint    TEXT NOT NULL DEFAULT 'https://api.anthropic.com',
    api_key     TEXT NOT NULL DEFAULT '',
    ctx_window  TEXT NOT NULL DEFAULT '200K',
    temperature REAL NOT NULL DEFAULT 0.3,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS agents (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    name_en       TEXT NOT NULL DEFAULT '',
    role          TEXT NOT NULL DEFAULT '',
    color         TEXT NOT NULL DEFAULT '#e8772e',
    initial       TEXT NOT NULL DEFAULT '?',
    llm_id        TEXT REFERENCES llm_configs(id),
    system_prompt TEXT NOT NULL DEFAULT '',
    forge_role    TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    repo_path   TEXT NOT NULL DEFAULT '',
    branch_dev  TEXT NOT NULL DEFAULT 'dev',
    branch_main TEXT NOT NULL DEFAULT 'main',
    status      TEXT NOT NULL DEFAULT 'active',
    config_yaml TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS issues (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id),
    source_type TEXT NOT NULL DEFAULT 'manual',
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category    TEXT NOT NULL DEFAULT 'Feature',
    severity    TEXT NOT NULL DEFAULT 'medium',
    priority    INTEGER,
    status      TEXT NOT NULL DEFAULT 'pending_analysis',
    fingerprint TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_issues_project ON issues(project_id);
CREATE INDEX IF NOT EXISTS ix_issues_status  ON issues(status);

CREATE TABLE IF NOT EXISTS issue_analyses (
    id                  TEXT PRIMARY KEY,
    issue_id            TEXT NOT NULL UNIQUE REFERENCES issues(id),
    authenticity_score  REAL NOT NULL DEFAULT 0,
    feasibility_score   REAL,
    priority_suggestion INTEGER,
    category_suggestion TEXT,
    severity_suggestion TEXT,
    duplicate_of        TEXT REFERENCES issues(id),
    affected_modules    TEXT,
    analysis_summary    TEXT NOT NULL DEFAULT '',
    raw_llm_output      TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS change_requests (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(id),
    issue_id            TEXT NOT NULL REFERENCES issues(id),
    status              TEXT NOT NULL DEFAULT 'pending_execution',
    admin_id            TEXT,
    approved_at         TEXT,
    admin_suggestions_1 TEXT,
    admin_suggestions_2 TEXT,
    target_branch       TEXT NOT NULL DEFAULT 'dev',
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_cr_project ON change_requests(project_id);
CREATE INDEX IF NOT EXISTS ix_cr_status  ON change_requests(status);

CREATE TABLE IF NOT EXISTS worktree_sessions (
    id                TEXT PRIMARY KEY,
    change_request_id TEXT NOT NULL REFERENCES change_requests(id),
    worktree_path     TEXT NOT NULL DEFAULT '',
    branch_name       TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'pending',
    prompt_snapshot   TEXT,
    iteration_count   INTEGER NOT NULL DEFAULT 1,
    report_content    TEXT,
    started_at        TEXT,
    completed_at      TEXT
);

CREATE TABLE IF NOT EXISTS conversations (
    id         TEXT PRIMARY KEY,
    type       TEXT NOT NULL DEFAULT 'direct',
    name       TEXT,
    color      TEXT NOT NULL DEFAULT '#e8772e',
    initial    TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS conversation_members (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    agent_id        TEXT NOT NULL REFERENCES agents(id),
    PRIMARY KEY (conversation_id, agent_id)
);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    from_agent      TEXT,
    content_json    TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_messages_conv ON messages(conversation_id, created_at);

CREATE TABLE IF NOT EXISTS job_executions (
    id              TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    job_type        TEXT NOT NULL,
    payload         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    attempt         INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    enqueued_at     TEXT NOT NULL DEFAULT (datetime('now')),
    started_at      TEXT,
    completed_at    TEXT,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_job_status ON job_executions(status);
