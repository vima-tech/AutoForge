-- Node 07: security audit results (design §4.3, pipeline security node)
CREATE TABLE IF NOT EXISTS security_audits (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    change_request_id TEXT,
    status            TEXT NOT NULL DEFAULT 'running',  -- running | passed | flagged | failed
    severity          TEXT NOT NULL DEFAULT 'none',     -- none | low | medium | high | critical
    summary           TEXT NOT NULL DEFAULT '',
    findings_json     TEXT NOT NULL DEFAULT '[]',        -- TEXT JSON array of findings
    issues_created    TEXT NOT NULL DEFAULT '[]',        -- TEXT JSON array of issue ids
    started_at        TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_security_audits_project ON security_audits(project_id);
CREATE INDEX IF NOT EXISTS idx_security_audits_cr ON security_audits(change_request_id);
