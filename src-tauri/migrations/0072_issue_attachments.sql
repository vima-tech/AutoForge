-- 需求条目附件（图片/文档）：镜像 conversation_attachments（0008），仅把
-- conversation_id 换成 issue_id。供需求录入（速录/表单）与需求审核挂载截图/附件，
-- 图片附件可经 supports_vision 的分析 Agent 内联识别。
CREATE TABLE IF NOT EXISTS issue_attachments (
    id              TEXT PRIMARY KEY,
    issue_id        TEXT NOT NULL REFERENCES issues(id),
    original_name   TEXT NOT NULL,
    stored_name     TEXT NOT NULL,
    rel_path        TEXT NOT NULL UNIQUE,
    mime            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    sha256          TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_issue_attachments_issue
    ON issue_attachments(issue_id, created_at);
