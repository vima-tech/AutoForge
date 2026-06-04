ALTER TABLE agents ADD COLUMN role_type TEXT NOT NULL DEFAULT 'business';
ALTER TABLE agents ADD COLUMN system_kind TEXT;
ALTER TABLE agents ADD COLUMN capabilities_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agents ADD COLUMN max_concurrency INTEGER NOT NULL DEFAULT 1;
ALTER TABLE agents ADD COLUMN visible_in_chat INTEGER NOT NULL DEFAULT 1;
ALTER TABLE agents ADD COLUMN mentionable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE agents ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;

INSERT OR IGNORE INTO agents (
    id,
    name,
    name_en,
    role,
    color,
    initial,
    llm_id,
    system_prompt,
    forge_role,
    role_type,
    system_kind,
    capabilities_json,
    max_concurrency,
    visible_in_chat,
    mentionable,
    enabled
) VALUES (
    'agent-system-planner',
    '调度器',
    'Planner',
    '自然语言任务解析 · 多 Agent 编排',
    '#6f7d91',
    '调',
    COALESCE(
        (SELECT id FROM llm_configs WHERE id = 'llm-sonnet4'),
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的系统调度 Agent。你的职责是把用户在群聊中的自然语言请求转换为严格 JSON 编排计划。只输出 JSON，不输出 Markdown。只允许选择候选列表中的 Agent，不得虚构 Agent。默认把多个被点名的 Agent 放入 parallel 步骤；当用户要求总结、裁决、汇总或产出文档时，在后续添加 single 步骤。',
    NULL,
    'system',
    'planner',
    '["planning","routing","orchestration"]',
    1,
    0,
    0,
    1
);

CREATE TABLE IF NOT EXISTS conversation_tasks (
    id                 TEXT PRIMARY KEY,
    conversation_id    TEXT NOT NULL REFERENCES conversations(id),
    trigger_message_id TEXT NOT NULL REFERENCES messages(id),
    planner_agent_id   TEXT REFERENCES agents(id),
    instruction        TEXT NOT NULL DEFAULT '',
    plan_json          TEXT NOT NULL DEFAULT '{}',
    status             TEXT NOT NULL DEFAULT 'pending',
    error              TEXT,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    started_at         TEXT,
    completed_at       TEXT
);

CREATE INDEX IF NOT EXISTS ix_conversation_tasks_conv
    ON conversation_tasks(conversation_id, created_at DESC);
CREATE INDEX IF NOT EXISTS ix_conversation_tasks_status
    ON conversation_tasks(status, created_at DESC);

CREATE TABLE IF NOT EXISTS conversation_task_steps (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL REFERENCES conversation_tasks(id),
    step_index     INTEGER NOT NULL,
    step_type      TEXT NOT NULL DEFAULT 'single',
    agent_ids_json TEXT NOT NULL DEFAULT '[]',
    instruction    TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'pending',
    error          TEXT,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    started_at     TEXT,
    completed_at   TEXT
);

CREATE INDEX IF NOT EXISTS ix_conversation_task_steps_task
    ON conversation_task_steps(task_id, step_index);

CREATE TABLE IF NOT EXISTS conversation_task_runs (
    id           TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL REFERENCES conversation_tasks(id),
    step_id      TEXT NOT NULL REFERENCES conversation_task_steps(id),
    agent_id     TEXT NOT NULL REFERENCES agents(id),
    status       TEXT NOT NULL DEFAULT 'pending',
    started_at   TEXT,
    completed_at TEXT,
    message_id   TEXT REFERENCES messages(id),
    output_text  TEXT,
    error        TEXT
);

CREATE INDEX IF NOT EXISTS ix_conversation_task_runs_task
    ON conversation_task_runs(task_id);
CREATE INDEX IF NOT EXISTS ix_conversation_task_runs_step
    ON conversation_task_runs(step_id);
CREATE INDEX IF NOT EXISTS ix_conversation_task_runs_agent
    ON conversation_task_runs(agent_id, status);
