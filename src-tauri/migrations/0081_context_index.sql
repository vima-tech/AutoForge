-- 上下文基质 L2：ContextItem 薄索引（方法论平台/基质设计 §3.1、§6）。
--
-- 目的：把当前「游离在上下文之外」的一切物料 + 过程信息（物料库、编码 Agent 日志、
-- llm_trace、会议室消息、CR 审核意见、孵化台草稿……）统一投影为「可被任意环节按需
-- 取用的上下文条目」的元数据索引。正文永远回原表/文件懒取（content_ref 定位），
-- 本表**不存正文**——是读侧统一视图 + 一层薄索引，不破坏任何既有存储与迁移。
--
-- 铁律：纯新增表，不改任何既有表语义（对齐「迁移只增不改」）。各来源写入时顺带登记
-- （register），或后台投影任务补齐。trust 决定回灌上下文前是否过 has_obvious_injection。
CREATE TABLE IF NOT EXISTS context_index (
    id           TEXT PRIMARY KEY,               -- 稳定引用：<source_kind>:<source_id> 派生，跨会话/跨阶段有效
    project_id   TEXT NOT NULL,                  -- 归属项目（跨项目隔离边界）
    source_kind  TEXT NOT NULL,                  -- file_priority/workspace_doc/material/chat_message/code_agent_log/llm_trace/incubator_draft/...
    source_id    TEXT NOT NULL,                  -- 原表主键 / 文件相对路径
    title        TEXT NOT NULL DEFAULT '',       -- 人可读标题
    origin_stage TEXT NOT NULL DEFAULT '',       -- requirement/design/chat/coding/review/ops
    origin_actor TEXT NOT NULL DEFAULT '',       -- user / agent-id / system
    content_ref  TEXT NOT NULL DEFAULT '',       -- 正文定位器：file:<path> / table:<t>.<col>#<id> / lazy:<kind>:<id>
    size_hint    INTEGER NOT NULL DEFAULT 0,     -- 体积（字节，装配预算用）
    trust        TEXT NOT NULL DEFAULT 'trusted',-- trusted / external_untrusted（外部来源必过注入闸）
    labels       TEXT NOT NULL DEFAULT '[]',     -- 自由标签 JSON 数组（检索 / 取景框过滤）
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 按项目 + 来源类型 + 时间倒序枚举候选（装配引擎的主查询路径）。
CREATE INDEX IF NOT EXISTS idx_context_index_project
    ON context_index (project_id, source_kind, created_at DESC);

-- 按原始来源定位（投影登记时的 upsert / 反查）。
CREATE INDEX IF NOT EXISTS idx_context_index_source
    ON context_index (source_kind, source_id);
