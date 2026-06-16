use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

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
}

pub fn emit(app: &AppHandle, event: AppEvent) {
    let _ = app.emit("AutoForge://event", event);
}
