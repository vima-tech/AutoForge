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

/// 所有字段均为 Clone（Db/JobSender 廉价克隆，其余走 `Arc` 共享底层资源），故整体可
/// `#[derive(Clone)]`——供 `core::rpc::Ctx` 在 dispatch 边界从托管 state 克隆出 `Arc<AppState>`，
/// 克隆后各 `Arc` 字段仍指向同一份共享资源（Mutex/信号量/子进程表不分叉）。
#[derive(Clone)]
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

/// 按 change_request_id 的「CR worktree 互斥锁」。所有会写同一个 CR worktree 的操作
/// （`merge::run`、`ai_resolve_conflict`、人工/外部解冲突、`revert::run`）持此锁，
/// 避免并发 git 在同一 worktree 互踩（index.lock 冲突 / 半完成合并）。与 `merge_lock`
/// 的加锁顺序固定为「先 merge_lock 后 cr_lock」，避免死锁。
static CR_LOCKS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::OnceLock::new();

/// 取（或惰性创建）某 CR 的 worktree 互斥锁。
pub fn cr_lock(cr_id: &str) -> Arc<Mutex<()>> {
    let map = CR_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    guard
        .entry(cr_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 按 conversation_id 的「会话串行锁」。同一会话内会读-改消息流的写操作——
/// 上下文压缩/结论（`compress_context_now`）与会话任务编排（`execute_conversation_task`
/// 的创建/状态流转）——持此锁，避免同一会话并发交织导致：摘要重复插入、原消息被
/// 重复排除、同一会话并发跑多个 running 任务。跨会话仍并行（粒度=会话）。
/// 走进程内全局 OnceLock，保持纯 Rust、零 Tauri 依赖（与 merge_lock/cr_lock 同构）。
static CONVERSATION_LOCKS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::OnceLock::new();

/// 取（或惰性创建）某会话的串行锁。
pub fn conversation_lock(conversation_id: &str) -> Arc<Mutex<()>> {
    let map = CONVERSATION_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    guard
        .entry(conversation_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 全局「agents 写锁」：`agents` 是规模很小的全局配置表，角色分配/新增/删除都是
/// 「先读快照 → 在 Rust 里算新值 → 写回」的读-改-写。并发调用会基于同一过期快照各自计算，
/// 导致角色被覆盖丢失、重复插入。用一把全局互斥串行化所有 agents 写操作即可彻底消除
/// 这些 stale-read / 重复插入竞态（写频率极低，串行无性能损失）。
static AGENTS_WRITE_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

/// 取全局 agents 写锁。
pub fn agents_write_lock() -> &'static Mutex<()> {
    AGENTS_WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

/// 全局「构建/测试池」信号量：所有合并门测试（`tasks/testing.rs::run_and_gate`）
/// 占一个许可，跨项目/CR 共享，限制任意时刻并发编译/测试数（默认 2），避免批量合并
/// 时多个 rustc/tsc 同时跑把 CPU 和内存打满。纯进程内、零 Tauri，全平台有效。
/// 大小在启动时按 `execution.build_slots` 初始化；未初始化时惰性回退为 2。
static BUILD_POOL: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
/// 当前构建池容量（用于 `set_build_slots` 计算增减许可数）。
static BUILD_SLOTS: std::sync::OnceLock<std::sync::Mutex<usize>> = std::sync::OnceLock::new();

/// 启动时按配置初始化构建池大小（幂等：仅首次生效）。
pub fn init_build_pool(slots: usize) {
    let n = slots.max(1);
    let _ = BUILD_POOL.set(Arc::new(tokio::sync::Semaphore::new(n)));
    let _ = BUILD_SLOTS.set(std::sync::Mutex::new(n));
}

/// 取全局构建池（未初始化则惰性建一个大小 2 的）。
pub fn build_pool() -> Arc<tokio::sync::Semaphore> {
    BUILD_POOL
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
}

/// 热改构建池容量（Settings 即时生效，与 max_slots/cpu_budget 一致）：
/// 增 → `add_permits` 立即放大；减 → 后台收回多余许可（等正在跑的编译让出后 forget），
/// 不阻塞调用方。
pub fn set_build_slots(slots: usize) {
    let n = slots.max(1);
    let sem = build_pool();
    let cur_lock = BUILD_SLOTS.get_or_init(|| std::sync::Mutex::new(2));
    let mut cur = cur_lock.lock().unwrap();
    if n > *cur {
        sem.add_permits(n - *cur);
        *cur = n;
    } else if n < *cur {
        let remove = (*cur - n) as u32;
        *cur = n;
        tokio::spawn(async move {
            if let Ok(p) = sem.acquire_many_owned(remove).await {
                p.forget();
            }
        });
    }
}

/// 运行中编码 Agent 的实时日志缓冲（按 cr_id）。前端中途进入「执行日志」时，realtime
/// `CodeAgentLog` 事件只能拿到订阅之后的增量；此缓冲自任务开始累计全文，供
/// `get_running_code_agent_log` 一次性回灌，使日志从头完整可见。append-only + 上限保护
/// （超 CAP 保留尾部 KEEP，与前端一致），任务结束 `running_log_clear` 清理（此后完整日志由
/// `code_agent_run_logs` 落库列表接管）。零 Tauri、进程内全局，非 Tauri 入口也能用。
struct RunningLog {
    text: String,
    /// 已追加的 chunk 数；每段事件携带各自序号（0-based），前端据此与快照去重，避免回灌重叠。
    seq: u64,
}
static RUNNING_LOGS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, RunningLog>>> =
    std::sync::OnceLock::new();
const RUNNING_LOG_CAP: usize = 400_000;
const RUNNING_LOG_KEEP: usize = 300_000;

/// 追加一段实时日志，返回该 chunk 的序号（0-based）。
pub fn running_log_append(cr_id: &str, chunk: &str) -> u64 {
    let map = RUNNING_LOGS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    let entry = guard
        .entry(cr_id.to_string())
        .or_insert_with(|| RunningLog {
            text: String::new(),
            seq: 0,
        });
    let seq = entry.seq;
    entry.seq += 1;
    entry.text.push_str(chunk);
    // 超上限则保留尾部 KEEP 字符（按 UTF-8 字符边界裁，避免切坏多字节字符）。
    if entry.text.len() > RUNNING_LOG_CAP {
        let mut cut = entry.text.len() - RUNNING_LOG_KEEP;
        while cut < entry.text.len() && !entry.text.is_char_boundary(cut) {
            cut += 1;
        }
        entry.text = entry.text.split_off(cut);
    }
    seq
}

/// 取某 CR 当前实时日志快照：(全文, 下一个 chunk 序号)。无缓冲（未运行/已清理）返回 ("", 0)。
pub fn running_log_snapshot(cr_id: &str) -> (String, u64) {
    let map = RUNNING_LOGS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let guard = map.lock().unwrap();
    match guard.get(cr_id) {
        Some(e) => (e.text.clone(), e.seq),
        None => (String::new(), 0),
    }
}

/// 任务结束时清理某 CR 的实时日志缓冲（完整日志已落库，realtime 缓冲不再需要）。
pub fn running_log_clear(cr_id: &str) {
    if let Some(map) = RUNNING_LOGS.get() {
        map.lock().unwrap().remove(cr_id);
    }
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

/// 共享依赖缓存根目录：与 worktrees 同级的 `dep-cache/`，按 lockfile 指纹分桶存放
/// 「以 origin/<dev> 为基点」的 node_modules，供所有 worktree 软链公用（见 core::dep_cache）。
pub fn dep_cache_base() -> String {
    let wt = worktrees_base();
    std::path::Path::new(&wt)
        .parent()
        .map(|p| p.join("dep-cache").to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{}-dep-cache", wt))
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
