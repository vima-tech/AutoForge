-- M10 反馈 widget 专用、可独立吊销的接入 token。
-- 与 intake_configs.webhook_token（GitHub 同步/通用 webhook 共用的主 token）解耦：
--   * 每个 token 绑定单一 project，只能为该项目投递反馈（webhook.rs 校验归属）；
--   * enabled=0 即吊销，删行亦可，互不影响主 token 与 GitHub 接入；
--   * 经 widget token 投递的反馈一律走 Triage 待整理池（不自动烧 AI 分析预算）。
-- token 明文存储：它本就会明文嵌入被嵌入页面的 HTML，对其加密无意义。
CREATE TABLE IF NOT EXISTS widget_tokens (
  id           TEXT PRIMARY KEY,
  project_id   TEXT NOT NULL,
  token        TEXT NOT NULL UNIQUE,
  label        TEXT NOT NULL DEFAULT '',
  enabled      INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL DEFAULT (datetime('now')),
  expires_at   TEXT,
  last_used_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_widget_tokens_token   ON widget_tokens(token);
CREATE INDEX IF NOT EXISTS idx_widget_tokens_project ON widget_tokens(project_id);
