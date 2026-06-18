-- 统一的「环节 Agent 结构化产出」存储（schema 驱动 agent 的落点）。
-- 每个流水线/编排环节 agent 按其版本化 schema 产出的结构化结果都沉淀到此一张表：
--   · 流水线级粒度（区别于 llm_traces 的单次调用级粒度）
--   · trace_id 链回 llm_traces，做「环节产出 → 那一步模型怎么想的」下钻
--   · role + schema_version 支撑「字段级体检 / 版本 A/B / 失败回灌」的持续优化循环
-- additive：不改动 issue_analyses / test_sessions 等既有表；analysis 在保留原表的同时 dual-write 到此。
CREATE TABLE IF NOT EXISTS agent_outputs (
    id             TEXT PRIMARY KEY,
    role           TEXT NOT NULL,            -- analysis | test | triage | proposer | ...
    schema_version TEXT NOT NULL,            -- 该 role schema 的版本，支撑版本对比/迁移
    target_kind    TEXT NOT NULL,            -- issue | cr | task | conversation
    target_id      TEXT NOT NULL,            -- 关联实体 id（按 target_kind 解释）
    project_id     TEXT,
    trace_id       TEXT,                     -- 链回 llm_traces（单步推理下钻），可空
    status         TEXT NOT NULL DEFAULT 'ok', -- ok | partial | error（解析/完备度状态，区别于业务结论）
    output_json    TEXT,                     -- 该 schema 的完整结构化产出（已截断）
    raw            TEXT,                     -- 原始模型文本（审计，已截断）
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_outputs_target  ON agent_outputs(target_kind, target_id);
CREATE INDEX IF NOT EXISTS idx_agent_outputs_role    ON agent_outputs(role, schema_version);
CREATE INDEX IF NOT EXISTS idx_agent_outputs_trace   ON agent_outputs(trace_id);
CREATE INDEX IF NOT EXISTS idx_agent_outputs_project ON agent_outputs(project_id);
CREATE INDEX IF NOT EXISTS idx_agent_outputs_created ON agent_outputs(created_at DESC);
