//! 远程 dev 依赖缓存：为「以 origin/<dev> 为基点」的 worktree 准备一份与该基点 lockfile
//! 完全匹配的 `node_modules`，按 lockfile 指纹做 key 在数据目录下集中缓存，供所有 worktree
//! 软链公用。
//!
//! 解决的漂移问题：worktree 代码取自 `origin/<dev>`，但若沿用「软链主仓 node_modules」
//! （那份是按*本地* dev 安装的），当 origin/dev 新增/升级了依赖时，worktree 里 import
//! 新包会 module not found。这里改为按 worktree（=origin/dev）自身的 lockfile 准备依赖，
//! 让代码与依赖都对齐 origin/dev，且同一指纹只装一次、全体 worktree 复用。
//!
//! 范围：仅根 `node_modules`（npm/pnpm/yarn/bun）。Rust `target` 等编译缓存不是 install
//! 产物，仍由 `stack::link_dep_caches` 软链主仓（按 dev 单独建会触发全量重编译，得不偿失）。
//!
//! 纯 Rust，零 Tauri。无 node 项目 / 缺包管理器 / 离线 / 安装失败一律返回 `None`，调用方
//! 回退到旧的「软链主仓」路径，保证零回归、绝不阻断编码。

use crate::core::platform;
use crate::state::dep_cache_base;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// per-key 串行锁：同一依赖指纹只装一次，避免并发 CR 同时 install 互踩。
fn lock_for(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
        std::sync::OnceLock::new();
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut m = map.lock().unwrap();
    m.entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 探测 worktree（其内容 = origin/<dev>）的包管理器与 lockfile 文件名；非 node 项目→None。
/// npm 可能没有 lock，则用 `package.json` 兜底作指纹源。
fn detect(worktree: &Path) -> Option<(&'static str, &'static str)> {
    if worktree.join("pnpm-lock.yaml").exists() {
        Some(("pnpm", "pnpm-lock.yaml"))
    } else if worktree.join("yarn.lock").exists() {
        Some(("yarn", "yarn.lock"))
    } else if worktree.join("bun.lockb").exists() {
        Some(("bun", "bun.lockb"))
    } else if worktree.join("package-lock.json").exists() {
        Some(("npm", "package-lock.json"))
    } else if worktree.join("package.json").exists() {
        Some(("npm", "package.json"))
    } else {
        None
    }
}

fn hash_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// 安装命令（在缓存目录里执行）。有 lock 时用可复现安装，避免与基点漂移。
fn install_script(pm: &str) -> &'static str {
    match pm {
        "pnpm" => "pnpm install --frozen-lockfile --prod=false",
        "yarn" => "yarn install --frozen-lockfile",
        "bun" => "bun install --frozen-lockfile",
        // npm：有 lock 用 `ci`（干净可复现）；无 lock 时 ci 失败，回退 install。
        _ => "npm ci || npm install",
    }
}

/// 确保存在一份匹配 `worktree`（=origin/dev）lockfile 的共享 `node_modules`，返回其路径供软链。
/// best-effort：任何失败返回 `None`，由调用方回退软链主仓。
pub async fn ensure_shared_node_modules(worktree: &Path) -> Option<PathBuf> {
    let (pm, lockfile) = detect(worktree)?;
    if !platform::has_executable(pm) {
        return None; // 没装该包管理器 → 回退软链主仓
    }
    let lock_bytes = tokio::fs::read(worktree.join(lockfile)).await.ok()?;
    let key = format!("{}-{}", pm, &hash_hex(&lock_bytes)[..16]);

    let base = dep_cache_base();
    let dir = Path::new(&base).join(&key);
    let nm = dir.join("node_modules");

    // 快路径：已就绪直接用（`.ready` 完成标记 + node_modules 实在，杜绝半成品被软链）。
    if dir.join(".ready").exists() && nm.exists() {
        touch(&dir).await; // 续期 LRU
        spawn_gc();
        return Some(nm);
    }

    let guard = lock_for(&key);
    let _g = guard.lock().await;
    // 双检：等锁期间别的任务可能已装好。
    if dir.join(".ready").exists() && nm.exists() {
        touch(&dir).await;
        spawn_gc();
        return Some(nm);
    }

    // 装到独占 tmp 目录，成功后原子改名到正式 key 目录，避免半成品对外可见。
    tokio::fs::create_dir_all(&base).await.ok();
    let tmp = Path::new(&base).join(format!("{}.tmp.{}", key, std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    if tokio::fs::create_dir_all(&tmp).await.is_err() {
        return None;
    }

    // 复制安装所需 manifest（package.json + lock + 可选 .npmrc / pnpm workspace）。
    for f in ["package.json", lockfile, ".npmrc", "pnpm-workspace.yaml"] {
        let src = worktree.join(f);
        if src.exists() {
            let _ = tokio::fs::copy(&src, tmp.join(f)).await;
        }
    }
    if !tmp.join("package.json").exists() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return None;
    }

    info!(
        "dep_cache: installing shared node_modules (key={}, pm={})",
        key, pm
    );
    let mut cmd = platform::shell(install_script(pm));
    cmd.current_dir(&tmp);
    cmd.kill_on_drop(true); // 超时 / 提前返回时杀掉安装进程，不留挂起的 install
    let ok = match tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output()).await {
        Ok(Ok(out)) => out.status.success(),
        _ => false, // 超时或 spawn 失败
    };
    if !ok || !tmp.join("node_modules").exists() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        info!(
            "dep_cache: install failed (key={}), fallback to repo symlink",
            key
        );
        return None;
    }

    // 原子落位：tmp → dir。期间若被并发任务装好（dir 已存在）则丢弃自己的 tmp。
    if dir.exists() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    } else if tokio::fs::rename(&tmp, &dir).await.is_err() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return None;
    }
    let _ = tokio::fs::write(dir.join(".ready"), b"1").await;
    spawn_gc();
    Some(dir.join("node_modules"))
}

/// 续期：刷新 `.ready` 的修改时间作为 LRU 依据（命中/新建时调用）。
async fn touch(dir: &Path) {
    let _ = tokio::fs::write(dir.join(".ready"), b"1").await;
}

/// 后台触发一次 best-effort GC（不阻塞编码路径）。
fn spawn_gc() {
    tokio::spawn(async {
        gc_stale().await;
    });
}

/// 保留的最近使用桶数：低于此数不论新旧都留，避免频繁重装常用基点。
const GC_RETAIN: usize = 6;
/// 超过此空闲时长且超出保留数的桶才允许回收（双保险，给挂起任务留窗口）。
const GC_IDLE_SECS: u64 = 24 * 3600;

/// best-effort GC：清理不再被任何活跃 worktree 软链、且较久未使用的旧依赖桶，防止
/// origin/dev 多次升级依赖后缓存无限膨胀。**绝不删除仍被软链占用的桶**——否则正在
/// 执行的编码/测试会因 node_modules 中途消失而崩溃。
async fn gc_stale() {
    let base = dep_cache_base();
    let in_use = in_use_targets().await;

    // 收集就绪桶及其最近使用时间（.ready 的 mtime）。
    let mut buckets: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    let mut rd = match tokio::fs::read_dir(&base).await {
        Ok(r) => r,
        Err(_) => return,
    };
    while let Ok(Some(ent)) = rd.next_entry().await {
        let p = ent.path();
        let ready = p.join(".ready");
        if !tokio::fs::try_exists(&ready).await.unwrap_or(false) {
            continue; // tmp / 半成品目录不碰
        }
        let mtime = tokio::fs::metadata(&ready)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);
        buckets.push((p, mtime));
    }

    buckets.sort_by(|a, b| b.1.cmp(&a.1)); // 最近使用在前
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(GC_IDLE_SECS);
    for (i, (dir, mtime)) in buckets.iter().enumerate() {
        let nm = dir.join("node_modules");
        if in_use.iter().any(|t| t == &nm || t.starts_with(dir)) {
            continue; // 仍被活跃 worktree 软链占用 → 绝不删
        }
        if i < GC_RETAIN {
            continue; // 最近 GC_RETAIN 个保留
        }
        if *mtime > cutoff {
            continue; // 太新，再等等
        }
        if tokio::fs::remove_dir_all(dir).await.is_ok() {
            info!("dep_cache: GC removed stale bucket {}", dir.display());
        }
    }
}

/// 扫 worktrees_base 下每个 worktree 的 `node_modules` 软链目标，得到「正被占用」的桶集合。
async fn in_use_targets() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let wt_base = crate::state::worktrees_base();
    let mut rd = match tokio::fs::read_dir(&wt_base).await {
        Ok(r) => r,
        Err(_) => return out,
    };
    while let Ok(Some(ent)) = rd.next_entry().await {
        let nm = ent.path().join("node_modules");
        if let Ok(target) = tokio::fs::read_link(&nm).await {
            out.push(target);
        }
    }
    out
}

/// 把共享 node_modules 软链进 worktree（仅 unix；node_modules 已存在则不动）。
/// 调用方应在 `stack::link_dep_caches` *之前* 调用——后者见 node_modules 已存在即跳过，
/// 只补软链 `target` 等其余缓存目录。
#[cfg(unix)]
pub fn link_into_worktree(shared_nm: &Path, worktree: &Path) {
    let dst = worktree.join("node_modules");
    if !dst.exists() {
        let _ = std::os::unix::fs::symlink(shared_nm, &dst);
    }
}

#[cfg(not(unix))]
pub fn link_into_worktree(_shared_nm: &Path, _worktree: &Path) {}
