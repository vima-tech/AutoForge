-- Generalize the Innate recall→outcome feedback loop beyond the code-execution
-- path. Key recall traces by (scope_kind, target_id) so multiple pipeline stages
-- share one mechanism:
--   scope_kind='issue'           → analysis recall, consumed at review_1
--                                   (approve = success / reject = failure)
--   scope_kind='change_request'  → code recall, consumed at merge (success) /
--                                   review_2 reject / terminal execution failure
-- The row is upserted at recall time and deleted when its outcome is recorded.
CREATE TABLE IF NOT EXISTS kb_recall_traces (
  scope_kind  TEXT NOT NULL,            -- 'issue' | 'change_request'
  target_id   TEXT NOT NULL,            -- issue_id or change_request_id
  project_id  TEXT NOT NULL,
  trace_id    TEXT NOT NULL,
  used_ids    TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (scope_kind, target_id)
);

-- Carry over any in-flight code-stage traces from the single-purpose table, then
-- retire it so the whole loop runs through one table.
INSERT OR IGNORE INTO kb_recall_traces (scope_kind, target_id, project_id, trace_id, used_ids, created_at)
  SELECT 'change_request', change_request_id, project_id, trace_id, used_ids, created_at FROM kb_traces;

DROP TABLE IF EXISTS kb_traces;
