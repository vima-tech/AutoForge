//! 协作式「CPU 核预算」加权令牌池（纯 Rust，零 Tauri）。
//!
//! 与内核层 [`crate::core::cpubudget`]（cgroup v2 硬兜底）分工，构成设计文档所述**两层**：
//! - **本模块（协作式令牌）**：合并门测试（`tasks/testing.rs::run_and_gate`）在进入每个
//!   计算相（tsc/lint/test/build check）前按**权重**借令牌、跑完归还——即「相位作用域租约」。
//!   令牌以「核」计数（默认 = nproc）。这封住跨 CR/项目的验证尖峰惊群，把同时撞上的编译
//!   排成流水线（文档 §2.2/§4）。
//! - **cgroup（强制层）**：令牌是君子协定（依赖权重声明诚实）；cgroup 从内核把总 CPU 钉死，
//!   兜住撒谎权重与不透明 `claude -p` 内部的子进程扇出（文档 §5.2）。
//!
//! 取代旧 `state.rs::build_pool`（那是本模块「1 CR = 1 permit」的简化前身）。
//! 设计文档以 Redis 加权信号量 + Lua 原子 + FIFO 队列描述（§9）；其在**进程内**的等价最优解
//! 就是 [`tokio::sync::Semaphore::acquire_many_owned`]——本身原子、FIFO 公平、多令牌一次获取，
//! 且 permit 随 `Drop` 自动归还（比手动 release 更安全，天然覆盖 panic / 超时提前返回）。零
//! 外部依赖，不违反「禁 Redis」技术栈锁定。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 一个核预算池：加权信号量 + 当前总容量。结构体化（而非散落全局静态）便于单测用局部实例，
/// 不污染进程级全局 [`POOL`]。
struct Pool {
    sem: Arc<Semaphore>,
    /// 当前总令牌数：用于 `acquire` 的 clamp 上限与 `set_permits` 的增减计算。
    total: Mutex<usize>,
    /// 当前**真正被阻塞**在 `acquire` 上的等待者数（即时拿到令牌的不计）。= 核预算队列深度，
    /// 供背压判定（文档 §10）与可观测。
    waiting: AtomicUsize,
}

impl Pool {
    fn new(permits: usize) -> Self {
        let n = permits.max(1);
        Self {
            sem: Arc::new(Semaphore::new(n)),
            total: Mutex::new(n),
            waiting: AtomicUsize::new(0),
        }
    }

    /// 进入计算相：按权重借核令牌。权重 > 总预算时**夹到总预算**，允许独占整机运行，
    /// 绝不死锁（文档 §7）。tokio 信号量 FIFO 公平，天然防大权重饥饿，无需自写队列。
    /// 先 `try_acquire`：即时拿到就不计队列深度；拿不到才把自己计入 `waiting`（真队列深度）。
    async fn acquire(&self, weight: usize) -> CpuLease {
        let total = *self.total.lock().unwrap();
        let n = clamp_weight(weight, total);
        let permit = match self.sem.clone().try_acquire_many_owned(n as u32) {
            Ok(p) => p,
            Err(_) => {
                self.waiting.fetch_add(1, Ordering::Relaxed);
                let p = self
                    .sem
                    .clone()
                    .acquire_many_owned(n as u32)
                    .await
                    .expect("cpu_permits semaphore is never closed");
                self.waiting.fetch_sub(1, Ordering::Relaxed);
                p
            }
        };
        CpuLease {
            _permit: permit,
            granted: n,
        }
    }

    fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// 核预算队列深度：当前被阻塞等待令牌的计算相数。
    fn queue_depth(&self) -> usize {
        self.waiting.load(Ordering::Relaxed)
    }

    fn total(&self) -> usize {
        *self.total.lock().unwrap()
    }

    /// 热改容量：增 → `add_permits` 立即放大；减 → 后台收回多余令牌（等在跑的验证让出后
    /// `forget`），不阻塞调用方。与旧 `state::set_build_slots` 行为一致。
    fn set_permits(&self, permits: usize) {
        let n = permits.max(1);
        let mut cur = self.total.lock().unwrap();
        if n > *cur {
            self.sem.add_permits(n - *cur);
            *cur = n;
        } else if n < *cur {
            let remove = (*cur - n) as u32;
            *cur = n;
            let sem = self.sem.clone();
            tokio::spawn(async move {
                if let Ok(permit) = sem.acquire_many_owned(remove).await {
                    permit.forget();
                }
            });
        }
    }
}

/// 权重夹取：`[1, total]`。`total==0` 视为 1（绝不请求 0 令牌卡死，也绝不请求超预算永久阻塞）。
fn clamp_weight(weight: usize, total: usize) -> usize {
    weight.clamp(1, total.max(1))
}

/// 一次相位租约：持有 `granted` 个核令牌，`Drop` 时自动全部归还。
pub struct CpuLease {
    _permit: OwnedSemaphorePermit,
    /// 实际授予的令牌数（= 声明权重 clamp 到 `[1, total]`）。用于子进程扇出封顶。
    pub granted: usize,
}

static POOL: OnceLock<Pool> = OnceLock::new();

/// 逻辑 CPU 数（兜底 1）。核预算的默认容量来源。
pub fn nproc() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn pool() -> &'static Pool {
    POOL.get_or_init(|| Pool::new(nproc()))
}

/// 启动时按 `execution.cpu_permits` 初始化核预算容量（幂等：仅首次生效）。
pub fn init(permits: usize) {
    let _ = POOL.set(Pool::new(permits));
}

/// 进入计算相：按权重借核令牌（见 [`Pool::acquire`]）。
pub async fn acquire(weight: usize) -> CpuLease {
    pool().acquire(weight).await
}

/// 当前可用令牌数（可观测：`total - available` 近似即时占用）。
pub fn available() -> usize {
    pool().available()
}

/// 核预算队列深度：当前被阻塞等待令牌的计算相数（可观测 + 背压信号，文档 §10）。
pub fn queue_depth() -> usize {
    pool().queue_depth()
}

/// 当前总容量。
pub fn total() -> usize {
    pool().total()
}

/// 热改核预算容量（Settings 即时生效，无需重启）。
pub fn set_permits(permits: usize) {
    pool().set_permits(permits);
}

/// 计算步权重 = 该步预期并行度（文档 §4.2）。优先按命令里的工具特征判定（更准），check
/// 类别名作兜底。pytest/cargo/构建这类重扇出封顶到 4；tsc/lint 单线程记 1；mypy/audit 记 2。
pub fn weight_of(check_name: &str, command: &str) -> usize {
    let c = command.to_ascii_lowercase();
    if c.contains("pytest") {
        return 4;
    }
    if c.contains("cargo build") || c.contains("cargo test") || c.contains("cargo nextest") {
        return 4;
    }
    if c.contains("vite build")
        || c.contains("webpack")
        || c.contains("next build")
        || c.contains("turbo")
    {
        return 4;
    }
    if c.contains("mypy") {
        return 2;
    }
    if c.contains("cargo audit") {
        return 2;
    }
    if c.contains("tsc") || c.contains("eslint") || c.contains("ruff") || c.contains("prettier") {
        return 1;
    }
    // 命令未命中已知工具 → 按 check 类别兜底。
    match check_name {
        "unit" | "integration" => 4,
        "security" => 2,
        "typing" | "lint" => 1,
        _ => 1,
    }
}

/// 子进程扇出封顶（文档 §5.1，env 途径）：**不改用户命令字符串**（零破坏风险），只经环境
/// 变量给子进程工具链施加并行度上限——装了对应工具才生效，没装无害。与 cgroup 硬兜底互补：
/// env 让「记账诚实」（声明 weight=4 就真把 rayon/cargo/make 压到 4），cgroup 兜住不认 env
/// 的进程。返回 `(key, value)` 列表，由 `run_check` spawn 前注入子进程环境。
pub fn parallelism_env(granted: usize) -> Vec<(String, String)> {
    let g = granted.max(1).to_string();
    vec![
        ("MAKEFLAGS".to_string(), format!("-j{g}")),
        ("CARGO_BUILD_JOBS".to_string(), g.clone()),
        ("RAYON_NUM_THREADS".to_string(), g.clone()),
        ("GOMAXPROCS".to_string(), g.clone()),
        ("PYTEST_XDIST_AUTO_NUM_WORKERS".to_string(), g),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_of_prefers_command_tool_then_falls_back_to_name() {
        assert_eq!(weight_of("unit", "pytest -q"), 4);
        assert_eq!(weight_of("unit", "cargo test --all"), 4);
        assert_eq!(weight_of("typing", "npx tsc --noEmit"), 1);
        assert_eq!(weight_of("lint", "eslint ."), 1);
        assert_eq!(weight_of("typing", "mypy src"), 2);
        assert_eq!(weight_of("security", "cargo audit"), 2);
        assert_eq!(weight_of("build", "vite build"), 4);
        // 命令未命中工具 → 按 check 类别兜底。
        assert_eq!(weight_of("unit", "./run-tests.sh"), 4);
        assert_eq!(weight_of("typing", "./check.sh"), 1);
        assert_eq!(weight_of("weird", "./whatever.sh"), 1);
    }

    #[test]
    fn clamp_weight_never_zero_never_exceeds_total() {
        // 独占不死锁：权重 > 总预算 → 夹到总预算（独占整机），不会请求超额永久阻塞。
        assert_eq!(clamp_weight(4, 2), 2);
        assert_eq!(clamp_weight(10, 8), 8);
        // 至少 1 令牌，绝不请求 0。
        assert_eq!(clamp_weight(0, 8), 1);
        // total==0 兜底为 1。
        assert_eq!(clamp_weight(4, 0), 1);
        // 正常情形原样。
        assert_eq!(clamp_weight(1, 8), 1);
        assert_eq!(clamp_weight(4, 8), 4);
    }

    #[test]
    fn parallelism_env_caps_common_toolchains() {
        let env: std::collections::HashMap<_, _> = parallelism_env(4).into_iter().collect();
        assert_eq!(env.get("CARGO_BUILD_JOBS").map(String::as_str), Some("4"));
        assert_eq!(env.get("RAYON_NUM_THREADS").map(String::as_str), Some("4"));
        assert_eq!(env.get("MAKEFLAGS").map(String::as_str), Some("-j4"));
        assert_eq!(env.get("GOMAXPROCS").map(String::as_str), Some("4"));
        // granted 至少 1，绝不注入 0。
        let env0: std::collections::HashMap<_, _> = parallelism_env(0).into_iter().collect();
        assert_eq!(env0.get("CARGO_BUILD_JOBS").map(String::as_str), Some("1"));
    }

    /// 加权信号量的相位租约语义：用**局部** Pool（不碰全局 POOL，避免测试间污染）。
    /// total=8：一个 weight=4 与一个 weight=4 可并发（各占 4）；第三个 weight=4 排队，前者
    /// 归还后才拿到——即验证尖峰被排成流水线。
    #[tokio::test]
    async fn acquire_serializes_when_budget_exhausted() {
        let p = Pool::new(8);
        let a = p.acquire(4).await;
        let b = p.acquire(4).await;
        assert_eq!(a.granted, 4);
        assert_eq!(b.granted, 4);
        assert_eq!(p.available(), 0, "8 核已被两个 weight=4 占满");
        // 第三个此刻拿不到（预算耗尽）→ 短超时内应始终 Pending。
        let pending =
            tokio::time::timeout(std::time::Duration::from_millis(50), p.acquire(4)).await;
        assert!(
            pending.is_err(),
            "预算耗尽时第三个计算相必须排队，不得超额并发"
        );
        // 归还一个 → 预算恢复，第三个立即拿到。
        drop(a);
        let c = p.acquire(4).await;
        assert_eq!(c.granted, 4);
        let _ = b;
    }

    /// 单个权重 > 总预算：夹到总预算独占整机，不死锁。
    #[tokio::test]
    async fn oversized_weight_runs_exclusively_without_deadlock() {
        let p = Pool::new(2);
        let lease = p.acquire(4).await; // clamp 到 2
        assert_eq!(lease.granted, 2);
        assert_eq!(p.available(), 0);
    }

    /// 队列深度只计**真正被阻塞**的等待者：即时拿到令牌的不计；令牌耗尽后再来的计入，
    /// 释放后回落 0。这是 §10 背压判定的信号源。
    #[tokio::test]
    async fn queue_depth_counts_only_blocked_waiters() {
        let p = std::sync::Arc::new(Pool::new(1));
        let a = p.acquire(1).await; // 占满，即时拿到 → 不计队列
        assert_eq!(p.queue_depth(), 0);

        let p2 = p.clone();
        let h = tokio::spawn(async move {
            let _l = p2.acquire(1).await; // 拿不到 → 阻塞，计入队列
            // 拿到后短暂持有再释放。
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        });
        // 等后台任务进入阻塞等待。
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(p.queue_depth(), 1, "一个被阻塞的等待者应计入队列深度");

        drop(a); // 释放 → 等待者被唤醒，计数回落。
        h.await.unwrap();
        assert_eq!(p.queue_depth(), 0, "等待者拿到令牌后队列深度归零");
    }
}
