-- Node 03: prototype design prompts (生成可用于 OpenDesign/Stitch/Claude Design 的设计提示词)
CREATE TABLE IF NOT EXISTS prototype_prompts (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    issue_id    TEXT,
    tool_target TEXT NOT NULL DEFAULT 'generic',  -- opendesign | stitch | claude_design | generic
    title       TEXT NOT NULL DEFAULT '',
    prompt      TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_prototype_prompts_project ON prototype_prompts(project_id);
