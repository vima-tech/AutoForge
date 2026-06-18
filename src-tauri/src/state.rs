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
    /// 实时 ASR 会话：session_id → 音频/控制发送端。
    pub asr_sessions: Arc<
        Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<crate::core::asr_realtime::AsrCtl>>>,
    >,
}

static WORKTREES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static ATTACHMENTS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MATERIALS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static KB_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn init_worktrees_base(path: String) {
    WORKTREES_BASE.set(path).ok();
}

pub fn init_attachments_base(path: String) {
    ATTACHMENTS_BASE.set(path).ok();
}

pub fn init_materials_base(path: String) {
    MATERIALS_BASE.set(path).ok();
}

/// 兜底目录：仅在未经 `init_*_base` 初始化（如非 Tauri 入口）时使用。走系统临时目录
/// 而非硬编码 `/tmp`，以便 Windows 也有合法路径。
fn temp_fallback(name: &str) -> String {
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .to_string()
}

pub fn worktrees_base() -> String {
    WORKTREES_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-worktrees"))
}

pub fn attachments_base() -> String {
    ATTACHMENTS_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-attachments"))
}

pub fn materials_base() -> String {
    MATERIALS_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-materials"))
}

pub fn init_kb_base(path: String) {
    KB_BASE.set(path).ok();
}

pub fn kb_base() -> String {
    KB_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-kb"))
}
