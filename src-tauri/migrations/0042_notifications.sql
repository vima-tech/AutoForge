-- 活动通知收件箱：把后端广播但前端落地即丢的 AppEvent 沉淀为可回看/可标记已读的活动流。
-- 仅持久化「有动作价值」的流水线事件（见 models/notification.rs draft_from_event 过滤），
-- 心跳类（TaskProgress/PipelineStatus/AsrResult）与已有独立角标覆盖的（MessageReceived 等）不入库。
CREATE TABLE IF NOT EXISTS notifications (
    id          TEXT PRIMARY KEY,
    category    TEXT NOT NULL,            -- intervene | progress | result | intake
    kind        TEXT NOT NULL,            -- 对应 AppEvent 变体的 snake_case type
    title       TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    link_page   TEXT,                     -- 前端跳转目标页（audit / delivery / chat …），可空
    link_ref    TEXT,                     -- 关联实体 id（cr_id / issue_id …），可空
    read        INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_unread  ON notifications(read, created_at DESC);
