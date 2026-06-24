use serde::{Deserialize, Serialize};

use crate::core::event::AppEvent;

/// 一条持久化的活动通知（收件箱条目）。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: String,
    /// 分类：intervene（需介入）/ progress（进度）/ result（结果）/ intake（录入）。
    pub category: String,
    /// 对应 AppEvent 变体的 snake_case type，便于前端按事件类型细化图标。
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// 前端跳转目标页（audit / delivery / chat …），可空。
    pub link_page: Option<String>,
    /// 关联实体 id（cr_id / issue_id …），可空。
    pub link_ref: Option<String>,
    pub read: bool,
    pub created_at: String,
}

/// 入库前的草稿（不含 id / created_at，由持久化层补齐）。
#[derive(Debug, Clone)]
pub struct NotificationDraft {
    pub category: &'static str,
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    pub link_page: Option<&'static str>,
    pub link_ref: Option<String>,
    /// 「需求线程」键：同一条需求的各阶段事件共享同一键，写入层据此 UPSERT 折叠为一行。
    /// 录入/分析阶段天然是 `issue_id`；CR 阶段（审核/测试/合并/审计）先填 `cr_id`，
    /// 由持久化层（`record_notification`）查 `change_requests` 解析回所属 `issue_id`，
    /// 使整条需求的全部阶段最终归并到同一线程。
    pub thread_key: Option<String>,
}

impl NotificationDraft {
    /// 把一个 AppEvent 映射成「值得沉淀」的通知草稿。
    ///
    /// 返回 `None` 表示该事件不入收件箱：
    /// - 心跳/高频类：`TaskProgress`、`PipelineStatus`、`WorktreeUpdate`、`PreviewUpdate`、`AsrResult`
    /// - 已有独立角标/页面覆盖、入库会淹没收件箱：`MessageReceived`、`ConversationTaskUpdated`
    ///
    /// 只保留对操作者有「回看/介入」价值的流水线生命周期事件。
    pub fn from_event(ev: &AppEvent) -> Option<NotificationDraft> {
        match ev {
            AppEvent::IssueCreated { issue_id, .. } => Some(NotificationDraft {
                category: "intake",
                kind: "issue_created",
                title: "新需求录入".into(),
                body: format!("需求 {issue_id} 已进入流水线"),
                link_page: Some("audit"),
                link_ref: Some(issue_id.clone()),
                thread_key: Some(issue_id.clone()),
            }),
            AppEvent::AnalysisCompleted { issue_id } => Some(NotificationDraft {
                category: "progress",
                kind: "analysis_completed",
                title: "需求分析完成".into(),
                body: format!("需求 {issue_id} 分析就绪，等待需求审核"),
                link_page: Some("audit"),
                link_ref: Some(issue_id.clone()),
                thread_key: Some(issue_id.clone()),
            }),
            AppEvent::ReviewNeeded {
                cr_id,
                issue_title,
                stage,
            } => Some(NotificationDraft {
                category: "intervene",
                kind: "review_needed",
                title: format!("需要审核 · 节点 {stage}"),
                body: issue_title.clone(),
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::IterationWarning {
                cr_id,
                iteration,
                soft_limit,
            } => Some(NotificationDraft {
                category: "intervene",
                kind: "iteration_warning",
                title: "迭代次数告警".into(),
                body: format!("{cr_id} 已迭代 {iteration} 轮（软上限 {soft_limit}），建议人工介入"),
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::TestCompleted {
                cr_id,
                status,
                summary,
                ..
            } => Some(NotificationDraft {
                category: "result",
                kind: "test_completed",
                title: if status == "passed" {
                    "测试通过".into()
                } else {
                    "测试失败".into()
                },
                body: if summary.is_empty() {
                    cr_id.clone()
                } else {
                    summary.clone()
                },
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::CrMerged { cr_id, .. } => Some(NotificationDraft {
                category: "result",
                kind: "cr_merged",
                title: "已合并到 dev".into(),
                body: cr_id.clone(),
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::CrReverted { cr_id, .. } => Some(NotificationDraft {
                category: "result",
                kind: "cr_reverted",
                title: "需求改动已撤销".into(),
                body: cr_id.clone(),
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::SecurityAuditCompleted {
                cr_id,
                severity,
                summary,
                ..
            } => Some(NotificationDraft {
                category: "result",
                kind: "security_audit_completed",
                title: format!("安全审计完成 · {severity}"),
                body: if summary.is_empty() {
                    cr_id.clone()
                } else {
                    summary.clone()
                },
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            AppEvent::MergeConflict { cr_id, files } => Some(NotificationDraft {
                category: "intervene",
                kind: "merge_conflict",
                title: "合并冲突待处理".into(),
                body: if files.is_empty() {
                    format!("{cr_id} 合并 dev 时冲突，需人工解决或 AI 解冲突")
                } else {
                    format!("{} 个文件冲突：{}", files.len(), files.join("、"))
                },
                link_page: Some("audit"),
                link_ref: Some(cr_id.clone()),
                thread_key: Some(cr_id.clone()),
            }),
            // 心跳 / 高频 / 已有独立角标覆盖 —— 不入收件箱。
            AppEvent::AutosupplyDegraded { reason } => Some(NotificationDraft {
                category: "intervene",
                kind: "autosupply_degraded",
                title: "自喂料深度提议器异常".into(),
                body: format!("proposer 连续多轮无产出，需介入：{reason}"),
                link_page: Some("settings"),
                link_ref: None,
                thread_key: Some("autosupply_degraded".into()),
            }),
            AppEvent::WorktreeUpdate { .. }
            | AppEvent::TaskProgress { .. }
            | AppEvent::PreviewUpdate { .. }
            | AppEvent::PipelineStatus { .. }
            | AppEvent::MessageReceived { .. }
            | AppEvent::ConversationTaskUpdated { .. }
            | AppEvent::AsrResult { .. }
            | AppEvent::AutosupplyStatus { .. }
            | AppEvent::CodeAgentLog { .. }
            | AppEvent::PreviewLog { .. }
            | AppEvent::CodingBriefChunk { .. } => None,
        }
    }
}
