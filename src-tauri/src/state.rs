use crate::core::concurrency::ConcurrencyManager;
use crate::db::Db;
use crate::tasks::runner::JobSender;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct DevServerHandle {
    pub child: Arc<Mutex<Option<tokio::process::Child>>>,
    pub url: String,
}

pub struct AppState {
    pub db: Db,
    pub job_tx: JobSender,
    pub concurrency: Arc<ConcurrencyManager>,
    pub dev_servers: Arc<Mutex<HashMap<String, DevServerHandle>>>,
    pub webhook_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

static WORKTREES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static ATTACHMENTS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MATERIALS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn init_worktrees_base(path: String) {
    WORKTREES_BASE.set(path).ok();
}

pub fn init_attachments_base(path: String) {
    ATTACHMENTS_BASE.set(path).ok();
}

pub fn init_materials_base(path: String) {
    MATERIALS_BASE.set(path).ok();
}

pub fn worktrees_base() -> String {
    WORKTREES_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| "/tmp/autoforge-worktrees".to_string())
}

pub fn attachments_base() -> String {
    ATTACHMENTS_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| "/tmp/autoforge-attachments".to_string())
}

pub fn materials_base() -> String {
    MATERIALS_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| "/tmp/autoforge-materials".to_string())
}
