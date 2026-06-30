-- 项目蓝图 2.0：把一次性「生成即弃」的蓝图升级为可持久、可多轮对话打磨的草稿态。
-- 草稿是单一真源（PRD + 规格 + 任务清单），AI 修正与人工手改都写同一份；
-- 满意后才 apply 落 .autoforge/ 工作区 + 入 triage 池（人审闸口不变）。
-- 每条规格/任务带稳定 id（生成时分配），供多轮修正时前端按 id diff 出改/增/删。

CREATE TABLE IF NOT EXISTS blueprint_drafts (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    brief         TEXT NOT NULL DEFAULT '',   -- 起草时的大需求原文
    prd_markdown  TEXT NOT NULL DEFAULT '',   -- 当前 PRD（Markdown）
    specs_json    TEXT NOT NULL DEFAULT '[]', -- Vec<BlueprintSpec> 序列化（含稳定 id）
    tasklist_json TEXT NOT NULL DEFAULT '[]', -- Vec<BlueprintTask> 序列化（含稳定 id）
    status        TEXT NOT NULL DEFAULT 'drafting', -- drafting / applied
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_blueprint_drafts_project
    ON blueprint_drafts(project_id, updated_at DESC);

-- 多轮修正的对话历史。assistant 行附 change_summary（这一轮改了什么的一句话）。
CREATE TABLE IF NOT EXISTS blueprint_messages (
    id             TEXT PRIMARY KEY,
    draft_id       TEXT NOT NULL,
    role           TEXT NOT NULL,             -- user / assistant
    content        TEXT NOT NULL DEFAULT '',
    change_summary TEXT NOT NULL DEFAULT '',  -- 仅 assistant：本轮变更摘要
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_blueprint_messages_draft
    ON blueprint_messages(draft_id, created_at);
