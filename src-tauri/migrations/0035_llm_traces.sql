-- LLM 调用链路追踪（LangSmith 式）。一次 Agent 调用 = 一个 trace_id；
-- 其下挂多个 span 行：root(kind=agent) + 每次模型 HTTP 调用(kind=llm) + 每次工具调用(kind=tool|mcp)。
-- 全部 best-effort 写入，失败不影响主流程。
CREATE TABLE IF NOT EXISTS llm_traces (
    id              TEXT PRIMARY KEY,
    trace_id        TEXT NOT NULL,            -- 同一次 Agent 调用的所有 span 共享
    parent_id       TEXT,                     -- 子 span 指向 root(=trace_id)；root 为 NULL
    seq             INTEGER NOT NULL DEFAULT 0,-- trace 内排序（root 用 -1 置顶）
    kind            TEXT NOT NULL,            -- agent | llm | tool | mcp
    name            TEXT,                     -- agent 名 / 模型名 / 工具名

    -- 关联维度（用于筛选）
    agent_id        TEXT,
    agent_role      TEXT,
    agent_name      TEXT,
    project_id      TEXT,
    issue_id        TEXT,
    conversation_id TEXT,
    task_id         TEXT,

    -- LLM 维度
    provider        TEXT,                     -- openai | anthropic
    model           TEXT,
    system_prompt   TEXT,

    -- 通用 I/O 与状态
    status          TEXT NOT NULL DEFAULT 'ok', -- ok | error
    error           TEXT,
    input           TEXT,                     -- 入参（messages / 工具 args，已截断）
    output          TEXT,                     -- 出参（回复 / 工具结果，已截断）

    -- 指标
    prompt_tokens     INTEGER,
    completion_tokens INTEGER,
    total_tokens      INTEGER,
    latency_ms        INTEGER,

    metadata_json   TEXT,                     -- 温度/迭代序号/tool_call_id 等补充
    started_at      TEXT,
    ended_at        TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_llm_traces_trace   ON llm_traces(trace_id, seq);
CREATE INDEX IF NOT EXISTS idx_llm_traces_created ON llm_traces(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_llm_traces_issue   ON llm_traces(issue_id);
CREATE INDEX IF NOT EXISTS idx_llm_traces_conv    ON llm_traces(conversation_id);
CREATE INDEX IF NOT EXISTS idx_llm_traces_role    ON llm_traces(agent_role);
CREATE INDEX IF NOT EXISTS idx_llm_traces_kind    ON llm_traces(kind);
