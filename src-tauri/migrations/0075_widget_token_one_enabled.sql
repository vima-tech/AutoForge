-- 修复：同一项目同一 mode 下并存多个 enabled token。
-- 根因：commands/widget.rs 的 ensure_token() 是非原子的「先查后插」，在 React StrictMode
--   双调用（IntakePanel 挂载时并发 getProjectWebhookToken）下两次调用都查到空、都 INSERT，
--   于是出现两条 created_at 相同的 enabled flow token；读回用 ORDER BY created_at DESC LIMIT 1
--   平局顺序未定义，导致面板/widget 展示的 token 与实际集成的那条对不上（"token 不对应"）。
-- 对策：① 去重，每组 (project_id, mode) 仅保留最新一条 enabled、其余吊销；
--       ② 部分唯一索引，从结构上保证「每个 (project_id, mode) 至多一条 enabled token」，
--          ensure_token 的并发后到者 INSERT 会被索引拒绝并回退复用赢家那条（见 widget.rs）。
-- 这样 get_project_webhook_token / get_widget_snippet 读回恒定唯一；项目轮换 token 后
-- widget 仍能通过 ensure_token 拿到当前有效（或自动新建）的那条 token。

-- ① 去重：保留每组 rowid 最大（最近插入）的 enabled 行，其余置 enabled=0。
--   重复行 created_at 相同，故按 rowid 取最新，结果确定。
UPDATE widget_tokens
SET enabled = 0
WHERE enabled = 1
  AND rowid NOT IN (
    SELECT MAX(rowid)
    FROM widget_tokens
    WHERE enabled = 1
    GROUP BY project_id, mode
  );

-- ② 部分唯一索引：每个 (project_id, mode) 至多一条 enabled。
CREATE UNIQUE INDEX IF NOT EXISTS idx_widget_tokens_one_enabled
  ON widget_tokens(project_id, mode) WHERE enabled = 1;
