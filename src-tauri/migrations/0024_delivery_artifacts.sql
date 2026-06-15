-- Delivery node artifacts — uploaded deliverables persisted under .autoforge/deliverables/
CREATE TABLE IF NOT EXISTS delivery_artifacts (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    node          TEXT NOT NULL DEFAULT 'general',  -- requirement|materials|prototype|spec|code|test|security|deploy|general
    original_name TEXT NOT NULL,
    stored_name   TEXT NOT NULL,
    rel_path      TEXT NOT NULL,                     -- relative to the project repo (.autoforge/deliverables/<node>/<stored>)
    mime          TEXT NOT NULL DEFAULT '',
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    sha256        TEXT NOT NULL DEFAULT '',
    description   TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_delivery_artifacts_project ON delivery_artifacts(project_id, node);
