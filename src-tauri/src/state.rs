use crate::core::concurrency::ConcurrencyManager;
use crate::db::Db;
use crate::tasks::runner::JobSender;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 可共享的子进程句柄：被开发预览服务器（dev_server / cr_preview）持有以便后续 kill。
/// 抽取为 type alias 避免在多处裸写嵌套类型（消除 clippy::type_complexity）。
pub type ChildHandle = Arc<Mutex<Option<tokio::process::Child>>>;

pub struct DevServerHandle {
    pub child: ChildHandle,
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
    /// 是否正有一轮自喂料在跑（手动「立即跑一轮」或周期调度共用）。状态真源在后端，
    /// 前端切页重挂载后查询此标志恢复回显，并据 `AutosupplyStatus` 事件实时跟随。
    pub autosupply_running: Arc<std::sync::atomic::AtomicBool>,
}

/// 按 project_id 的合并互斥锁表。同一项目的合并任务（`tasks/merge.rs::run`）全程持锁，
/// 串行执行，避免并发 `checkout dev && merge` 互相踩同一个 dev 工作树（竞态）；跨项目仍并行。
/// 走进程内全局而非 `AppState` 字段——保持纯 Rust、零 Tauri 依赖，非 Tauri 入口也能用。
static MERGE_LOCKS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::OnceLock::new();

/// 取（或惰性创建）某项目的合并锁。
pub fn merge_lock(project_id: &str) -> Arc<Mutex<()>> {
    let map = MERGE_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    guard
        .entry(project_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

static WORKTREES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static ATTACHMENTS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MATERIALS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static KB_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static BACKUPS_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static OPENDESIGN_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

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

pub fn init_backups_base(path: String) {
    BACKUPS_BASE.set(path).ok();
}

/// 配置备份导出目录（口令加密的 `.afbackup` 文件落此处）。
pub fn backups_base() -> String {
    BACKUPS_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-backups"))
}

pub fn init_opendesign_base(path: String) {
    OPENDESIGN_BASE.set(path).ok();
}

/// AutoForge 自管的 OpenDesign 检出目录：当本地探测不到已有检出时，
/// `git clone nexu-io/open-design` 落此处统一管理。
pub fn opendesign_base() -> String {
    OPENDESIGN_BASE
        .get()
        .cloned()
        .unwrap_or_else(|| temp_fallback("autoforge-opendesign"))
}
