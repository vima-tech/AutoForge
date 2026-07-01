use serde::Serialize;

/// 上下文基质 L2 的统一投影单元（方法论平台/基质设计 §3.1）。
///
/// 是对既有各表的**读侧薄索引**：正文永远回原表/文件经 `content_ref` 懒取，本结构只存
/// 元数据 + 定位引用。系统里一切「可以被喂给 Agent 的信息单元」都投影为一个 ContextItem，
/// 从而让**任何环节都能按需取用之前创建的一切上下文**（原则二）。
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ContextItem {
    /// 稳定引用（`<source_kind>:<source_id>` 派生），可被消息块/CR/任务持久引用，跨会话有效。
    pub id: String,
    /// 归属项目（跨项目隔离边界）。
    pub project_id: String,
    /// 来源类型（见 `core::context::source_kind`）。
    pub source_kind: String,
    /// 在原表中的主键 / 文件相对路径。
    pub source_id: String,
    /// 人可读标题。
    pub title: String,
    /// 产生阶段：requirement / design / chat / coding / review / ops。
    pub origin_stage: String,
    /// 产生者：user / agent-id / system。
    pub origin_actor: String,
    /// 正文定位器：`file:<path>` / `table:<t>.<col>#<id>` / `lazy:<kind>:<id>`。
    pub content_ref: String,
    /// 体积（字节，装配预算用）。
    pub size_hint: i64,
    /// 信任级别：`trusted` / `external_untrusted`（后者回灌前必过注入闸）。
    pub trust: String,
    /// 自由标签的 JSON 数组字符串（检索 / 取景框过滤）。
    pub labels: String,
    pub created_at: String,
    pub updated_at: String,
}
