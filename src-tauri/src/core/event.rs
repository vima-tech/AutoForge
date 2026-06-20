use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::models::notification::NotificationDraft;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    IssueCreated {
        issue_id: String,
        project_id: String,
    },
    AnalysisCompleted {
        issue_id: String,
    },
    WorktreeUpdate {
        cr_id: String,
        status: String,
        message: Option<String>,
    },
    /// Fine-grained progress heartbeat for a long-running CR task. Unlike
    /// `WorktreeUpdate` (which marks coarse status transitions), this reports the
    /// current phase within a stage so the UI shows life during the multi-minute
    /// claude CLI run instead of an opaque "executing".
    TaskProgress {
        cr_id: String,
        phase: String,
        note: Option<String>,
    },
    PreviewUpdate {
        cr_id: String,
        preview_id: String,
        status: String,
        preview_url: Option<String>,
    },
    TestCompleted {
        cr_id: String,
        test_session_id: String,
        status: String,
        summary: String,
    },
    ReviewNeeded {
        cr_id: String,
        issue_title: String,
        stage: u8,
    },
    CrMerged {
        cr_id: String,
        project_id: String,
    },
    SecurityAuditCompleted {
        cr_id: String,
        audit_id: String,
        severity: String,
        summary: String,
    },
    IterationWarning {
        cr_id: String,
        iteration: i64,
        soft_limit: i64,
    },
    PipelineStatus {
        active: usize,
        pending_review: usize,
        max_slots: usize,
    },
    MessageReceived {
        conversation_id: String,
        message_id: String,
    },
    ConversationTaskUpdated {
        conversation_id: String,
        task_id: String,
        status: String,
    },
    /// 实时 ASR 转写结果：增量(is_final=false)或整句(is_final=true)。
    AsrResult {
        session_id: String,
        text: String,
        is_final: bool,
    },
    /// 自喂料一轮的开始/结束。前端据此实时回显「运行中」状态——状态真源在后端，
    /// 切换页面后重挂载只需查询 `autosupply_is_running` 即可恢复，不再丢失回显。
    AutosupplyStatus {
        running: bool,
    },
    /// 合并前自动把 dev 并入 CR 分支时发生代码冲突：CR 已置 `merge_conflict` 态，
    /// 等待人工三方解决 / 一键重试 / AI 自动解冲突。
    MergeConflict {
        cr_id: String,
        files: Vec<String>,
    },
}

impl AppEvent {
    /// 若该事件是 CR（变更请求）维度的，返回其 `cr_id`。
    /// 通知收件箱据此把 CR 阶段事件归并回所属需求线程（见 `record_notification`）。
    pub fn cr_id(&self) -> Option<&str> {
        match self {
            AppEvent::WorktreeUpdate { cr_id, .. }
            | AppEvent::TaskProgress { cr_id, .. }
            | AppEvent::PreviewUpdate { cr_id, .. }
            | AppEvent::TestCompleted { cr_id, .. }
            | AppEvent::ReviewNeeded { cr_id, .. }
            | AppEvent::CrMerged { cr_id, .. }
            | AppEvent::SecurityAuditCompleted { cr_id, .. }
            | AppEvent::IterationWarning { cr_id, .. }
            | AppEvent::MergeConflict { cr_id, .. } => Some(cr_id),
            _ => None,
        }
    }
}

pub fn emit(app: &AppHandle, event: AppEvent) {
    let _ = app.emit("AutoForge://event", &event);

    // 在传输适配层（本文件是唯一的 Tauri 事件出口）顺手把「有动作价值」的事件
    // 沉淀进通知收件箱——这样所有 emit 调用点零改动即可获得持久化活动流。
    // 业务层仍只感知 `event::emit(app, AppEvent)`，不触碰持久化与 Tauri state。
    if NotificationDraft::from_event(&event).is_some() {
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            // 仅在已有 tokio 运行时上下文里 spawn（emit 始终在 Tauri 的 tokio 运行时内被调用）。
            if tokio::runtime::Handle::try_current().is_ok() {
                let db = state.db.clone();
                tokio::spawn(async move {
                    let _ = crate::commands::notifications::record_notification(&db, &event).await;
                });
            }
        }
    }
}
