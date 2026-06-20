-- 把 widget_tokens 泛化为「项目级需求接入 token」统一凭证表。
-- 背景：原入站 webhook 共用一个全局主 token（intake_configs.webhook_token），主 token
--   信任 payload 里的 project_id —— 任意持有者可往任意项目灌需求（跨项目伪造隐患）。
-- 改造：删除主 token 鉴权路径，所有入站 /webhook/issues 一律按本表的项目级 token 鉴权，
--   项目由 token 单向推导（不再信任 payload.project_id）。
-- 新增 mode 区分落库行为：
--   * flow   —— 可信集成（项目「需求入口」签发的 webhook token），自动进分析队列；
--   * triage —— 匿名反馈 widget，进待整理池，不自动消耗 AI 预算。
-- 旧反馈 widget token 默认 triage，保持原行为不变。
ALTER TABLE widget_tokens ADD COLUMN mode TEXT NOT NULL DEFAULT 'triage';
