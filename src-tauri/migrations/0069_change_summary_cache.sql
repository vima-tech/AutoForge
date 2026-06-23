-- AI 变更摘要缓存：摘要原为实时生成、不落库，导致每次切换需求/CR 都重跑 LLM。
-- 改为按 (cr_id) 缓存，并记录生成时所依据 diff 的内容哈希（diff_hash）。
-- 命中且 diff 未变（哈希一致）则直接返回缓存；diff 变化（重新编码/修改）哈希自动失效、重算覆盖。
-- 只缓存成功（status=ok）的结果；degraded/empty 不落库，以便下次自动重试。
CREATE TABLE IF NOT EXISTS change_summaries (
    cr_id        TEXT PRIMARY KEY,
    diff_hash    TEXT NOT NULL,         -- 生成所依据 diff 的 sha256（十六进制）
    summary_json TEXT NOT NULL,         -- 序列化的 ChangeSummary
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
