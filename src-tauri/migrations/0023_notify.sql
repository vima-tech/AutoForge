-- M11 — notification hub channels (design §13 通知)
CREATE TABLE IF NOT EXISTS notify_channels (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL DEFAULT '',
    kind       TEXT NOT NULL DEFAULT 'webhook',  -- slack | wecom | webhook
    target     TEXT NOT NULL DEFAULT '',          -- destination URL
    events     TEXT NOT NULL DEFAULT '',          -- CSV event filter; empty = all
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
