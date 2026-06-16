-- Innate record feedback loop: link a change request to the recall trace that
-- fed its coding prompt, so the eventual outcome (merge = success / review_2
-- reject = failure) can be written back via `innate record`. Latest execution
-- wins (upsert on change_request_id); the row is consumed on outcome.
CREATE TABLE IF NOT EXISTS kb_traces (
  change_request_id TEXT PRIMARY KEY,
  project_id        TEXT NOT NULL,
  trace_id          TEXT NOT NULL,
  used_ids          TEXT NOT NULL DEFAULT '',
  created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
