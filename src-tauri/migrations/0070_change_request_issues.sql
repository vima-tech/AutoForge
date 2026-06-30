-- 同文件多需求合并：一个变更请求（CR）可覆盖多个需求（issue）。
-- change_requests.issue_id 保留为「主需求」（驱动默认标题 / Innate 召回 / 旧 19 处调用点），
-- 本表承载 CR 的全部成员需求（含 primary），是「CR 的全部需求」的唯一真源。
CREATE TABLE IF NOT EXISTS change_request_issues (
  change_request_id TEXT NOT NULL,
  issue_id          TEXT NOT NULL,
  role              TEXT NOT NULL DEFAULT 'member',  -- 'primary' | 'member'
  sort_order        INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (change_request_id, issue_id)
);
CREATE INDEX IF NOT EXISTS idx_cri_cr    ON change_request_issues(change_request_id);
CREATE INDEX IF NOT EXISTS idx_cri_issue ON change_request_issues(issue_id);

-- 回填：每个既有 CR 写一条 primary 行，保证「CR 的全部需求」= 本表查询，新旧 CR 一致。
INSERT OR IGNORE INTO change_request_issues (change_request_id, issue_id, role, sort_order)
SELECT id, issue_id, 'primary', 0 FROM change_requests;
