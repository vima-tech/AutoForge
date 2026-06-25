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
    /// 已合并需求的改动被「撤销」（在 dev 上 `git revert` 了该 CR 的 squash 提交）。
    CrReverted {
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
    /// 自喂料的深度提议器（proposer）连续多轮失败（如模型工具调用格式不兼容、未绑定 LLM）。
    /// 进通知收件箱提醒操作者介入——否则 proposer 静默空转、工厂「发现不了问题」却无人知晓。
    AutosupplyDegraded {
        reason: String,
    },
    /// 合并前自动把 dev 并入 CR 分支时发生代码冲突：CR 已置 `merge_conflict` 态，
    /// 等待人工三方解决 / 一键重试 / AI 自动解冲突。
    MergeConflict {
        cr_id: String,
        files: Vec<String>,
    },
    /// 代码 Agent 运行期的实时日志增量（一段可读 stdout/stderr）。高频事件，
    /// 仅供 UI"实时滚动"——刻意不进通知收件箱（`from_event` 不匹配它），
    /// 也不归并到 CR 通知线程（不在 `cr_id()` 内），避免刷屏与持久化负担。
    CodeAgentLog {
        cr_id: String,
        phase: String,
        stream: String,
        chunk: String,
        /// 该 chunk 在本次运行内的序号（0-based）。前端中途进入时据此与
        /// `get_running_code_agent_log` 快照去重，无缝衔接已错过的开头与后续增量。
        seq: u64,
    },
    /// 预览/dev-server 启动日志的实时增量。`key` 与前端日志弹窗的 `sig` 对应
    /// （`cr:<id>` 或 `branch:<pid>:<branch>`），供前端只累积当前打开的那个日志。
    /// 由 `commands::cr_preview` 的文件 tail 任务发射（子进程输出直写文件、不流经 Rust，
    /// 故后端 tail 该文件增量再转成事件）。高频 UI-only：不进通知收件箱、不归并 CR 线程。
    PreviewLog {
        key: String,
        chunk: String,
    },
    /// 会议室「立即编码」AI 梳理需求时的流式思考增量。`conversation_id` 标识哪个会议室
    /// 的弹窗在等待，前端据此实时累积显示 AI 的思考过程，消除「干等」的等待感。
    /// 高频 UI-only：不进通知收件箱、不归并 CR 线程。
    CodingBriefChunk {
        conversation_id: String,
        chunk: String,
    },
    /// 会议室 Agent 回复时的「实时思考」增量：让等待中的会议室看到真实进展，消除干等。
    /// `run_id` 标识本次 Agent 执行（并行步骤里多个 Agent 同时发言时据此分流到各自的实时卡片）。
    /// `kind`：`token`=回复正文逐字增量 / `tool`=Agent 正在执行的工具动作（如「检索代码」）/
    /// `done`=该 Agent 已落库最终消息（前端据此撤下实时卡片，换成正式消息气泡）。
    /// `seq` 为本次执行内的递增序号，保证前端按序拼接。高频 UI-only：不进通知收件箱、不归并 CR 线程。
    AgentThinking {
        conversation_id: String,
        run_id: String,
        agent_id: String,
        agent_name: String,
        kind: String,
        text: String,
        seq: u64,
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
            | AppEvent::CrReverted { cr_id, .. }
            | AppEvent::SecurityAuditCompleted { cr_id, .. }
            | AppEvent::IterationWarning { cr_id, .. }
            | AppEvent::MergeConflict { cr_id, .. } => Some(cr_id),
            _ => None,
        }
    }
}

pub fn emit(app: &AppHandle, event: AppEvent) {
    let _ = app.emit("autoforge://event", &event);

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
