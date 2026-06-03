use crate::core::concurrency::ConcurrencyManager;
use crate::db::Db;
use crate::tasks::runner::JobSender;
use std::sync::Arc;

pub struct AppState {
    pub db: Db,
    pub job_tx: JobSender,
    pub concurrency: Arc<ConcurrencyManager>,
}

static WORKTREES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn init_worktrees_base(path: String) {
    WORKTREES_BASE.set(path).ok();
}

pub fn worktrees_base() -> String {
    WORKTREES_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| "/tmp/autoforge-worktrees".to_string())
}
