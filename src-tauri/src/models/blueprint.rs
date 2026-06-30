use serde::{Deserialize, Serialize};

/// 蓝图里的一条技术规格。`id` 为稳定标识（生成时分配），供多轮修正按 id 比对改/增/删。
/// `category` 期望落在宪法级集合（tech_stack/architecture/coding/api/testing），否则 apply 时回落 architecture。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintSpec {
    #[serde(default)]
    pub id: String,
    pub category: String,
    pub title: String,
    pub content_markdown: String,
}

/// 蓝图里的一条任务（→ 可一键转为 triage 需求）。`id` 同上为稳定标识。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintTask {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

pub fn default_category() -> String {
    "Feature".to_string()
}
pub fn default_severity() -> String {
    "medium".to_string()
}

/// 蓝图草稿——单一真源。AI 修正与人工手改都写这份；编码开发前不落 .autoforge、不入池。
/// 持久化时 specs/tasklist 序列化进 `*_json` 列，读出时再解析为下面的结构。
/// `status`：drafting（梳理中）/ coding（已编码开发）。派生执行工单经 issue_id/cr_id 关联。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintDraft {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub brief: String,
    pub prd_markdown: String,
    pub specs: Vec<BlueprintSpec>,
    pub tasklist: Vec<BlueprintTask>,
    pub status: String,
    pub issue_id: String,
    pub cr_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 大需求列表项（孵化台左栏）。`display_status` 由 status + 关联 CR 现状派生：
/// drafting / coding / in_review / implemented / failed / conflict。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintDraftSummary {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub display_status: String,
    pub spec_count: i64,
    pub task_count: i64,
    pub issue_id: String,
    pub cr_id: String,
    pub updated_at: String,
}

/// 多轮修正的一条对话消息。`change_summary` 仅 assistant 行有值。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlueprintMessage {
    pub id: String,
    pub draft_id: String,
    pub role: String,
    pub content: String,
    pub change_summary: String,
    pub created_at: String,
}

/// 草稿 + 对话历史的复合视图，供前端工作台一次性恢复全部状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintDraftView {
    pub draft: BlueprintDraft,
    pub messages: Vec<BlueprintMessage>,
}
