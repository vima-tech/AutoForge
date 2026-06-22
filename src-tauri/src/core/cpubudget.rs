//! cgroup v2 CPU 预算（Linux）：把所有 code agent 进程组 + 合并门测试塞进一个
//! AutoForge 专属子 cgroup，给它设 `cpu.max`，由内核把这些进程的【总 CPU】限到
//! `budget_pct% × nproc`。agent 照常跑 build/test（质量回路不动），只是被内核限速。
//!
//! 设计要点：
//! - **不禁测试**，只封顶——超预算的进程被 throttle，不是被杀。
//! - **agent 无关 + 不可逃逸**：罩在进程组外层，claude/codex/opencode/自研一视同仁，
//!   子孙进程（rustc/tsc/rg…）自动继承，无法绕过。
//! - **best-effort + 优雅降级**：任一步失败即整功能关闭、记日志，回退到 nice + 负载闸 +
//!   max_slots，绝不 panic、不阻断流水线。默认 `budget_pct=0` 即彻底关闭（零副作用）。
//! - 纯 Rust 写 cgroupfs，零 Tauri 类型。非 Linux 全是空操作。
//!
//! cgroup v2 的「no internal process」约束：一个 cgroup 不能既有进程、又给子组启用
//! 控制器。故启动时把【本进程】挪进 `…/autoforge.main`，给父组 `subtree_control` 开
//! `+cpu`，再建 `…/autoforge.agents` 写 `cpu.max`，agent/门的子进程都 attach 到它。

#[cfg(target_os = "linux")]
use std::sync::OnceLock;

/// 初始化好的 agents cgroup 目录（如 `/sys/fs/cgroup/<…>/autoforge.agents`）。
/// `None` = 未启用 / 不可用 → attach 全部空操作，回退到其它治理层。
#[cfg(target_os = "linux")]
static AGENTS_CG: OnceLock<Option<String>> = OnceLock::new();

/// 逻辑 CPU 数（兜底 1）。
fn nproc() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// 把「预算百分比 × nproc」换算成 cgroup `cpu.max` 的 `"<quota> <period>"` 串。
/// period 固定 100000µs；quota = pct/100 × nproc × period。pct<=0 或 >=100×nproc 视为不限。
pub fn cpu_max_line(pct: u64, nproc: usize) -> Option<String> {
    if pct == 0 {
        return None;
    }
    const PERIOD: u64 = 100_000;
    let quota = (pct as f64 / 100.0 * nproc as f64 * PERIOD as f64).round() as u64;
    if quota == 0 {
        return None;
    }
    Some(format!("{} {}", quota.max(1000), PERIOD))
}

/// 启动时调用（Linux）。`budget_pct=0` → 不做任何事。失败即静默禁用。
pub fn init(budget_pct: u64) {
    #[cfg(target_os = "linux")]
    {
        let result = try_init_linux(budget_pct);
        match &result {
            Some(path) => tracing::info!(
                "cpubudget: cgroup CPU 预算已启用（{}% × {} 核） → {}",
                budget_pct,
                nproc(),
                path
            ),
            None if budget_pct > 0 => tracing::info!(
                "cpubudget: 未能启用 cgroup CPU 预算（缺 cgroup v2 委派/权限），回退到 nice+负载闸"
            ),
            None => {}
        }
        let _ = AGENTS_CG.set(result);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = budget_pct;
    }
}

/// 把一个进程（及其子孙）纳入 CPU 预算。agent / 门的子进程 spawn 后调用。
/// 未启用或非 Linux 时空操作。
pub fn attach(pid: u32) {
    #[cfg(target_os = "linux")]
    {
        if pid == 0 {
            return;
        }
        if let Some(Some(cg)) = AGENTS_CG.get() {
            let _ = std::fs::write(format!("{}/cgroup.procs", cg), pid.to_string());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
    }
}

/// 热改预算（Settings 即时生效）。仅在已启用时有效。
pub fn set_budget(budget_pct: u64) {
    #[cfg(target_os = "linux")]
    {
        if let Some(Some(cg)) = AGENTS_CG.get() {
            let line = cpu_max_line(budget_pct, nproc()).unwrap_or_else(|| "max".to_string());
            let _ = std::fs::write(format!("{}/cpu.max", cg), line);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = budget_pct;
    }
}

#[cfg(target_os = "linux")]
fn try_init_linux(budget_pct: u64) -> Option<String> {
    if budget_pct == 0 {
        return None;
    }
    let cpu_max = cpu_max_line(budget_pct, nproc())?;

    // 解析本进程的 cgroup v2 路径：/proc/self/cgroup 里以 "0::" 开头那行。
    let content = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = content
        .lines()
        .find_map(|l| l.strip_prefix("0::"))?
        .trim()
        .to_string();
    if rel.is_empty() {
        return None;
    }
    let our_cg = format!("/sys/fs/cgroup{}", rel);

    // 须有可用的 cpu 控制器，否则白忙。
    let controllers = std::fs::read_to_string(format!("{}/cgroup.controllers", our_cg)).ok()?;
    if !controllers.split_whitespace().any(|c| c == "cpu") {
        return None;
    }

    let main_cg = format!("{}/autoforge.main", our_cg);
    let agents_cg = format!("{}/autoforge.agents", our_cg);
    std::fs::create_dir_all(&main_cg).ok()?;
    std::fs::create_dir_all(&agents_cg).ok()?;

    // no-internal-process：把本进程挪进 autoforge.main，腾空 our_cg，才能给子组开 +cpu。
    std::fs::write(format!("{}/cgroup.procs", main_cg), std::process::id().to_string()).ok()?;

    // 给父组子树启用 cpu 控制器（our_cg 此刻应已无内部进程；若仍有 webview 等兄弟进程
    // 占着，会写失败 → 返回 None 降级，本进程已在 main 子组中，无害）。
    std::fs::write(format!("{}/cgroup.subtree_control", our_cg), "+cpu").ok()?;

    // 设预算。写成功才算真正启用。
    std::fs::write(format!("{}/cpu.max", agents_cg), &cpu_max).ok()?;

    Some(agents_cg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_max_line_math() {
        // 0% / 0 核 → 不限。
        assert_eq!(cpu_max_line(0, 8), None);
        // 50% × 8 核 = 4 核 = 400000/100000。
        assert_eq!(cpu_max_line(50, 8).as_deref(), Some("400000 100000"));
        // 100% × 16 核 = 16 核。
        assert_eq!(cpu_max_line(100, 16).as_deref(), Some("1600000 100000"));
        // 75% × 4 核 = 3 核。
        assert_eq!(cpu_max_line(75, 4).as_deref(), Some("300000 100000"));
    }

    #[test]
    fn attach_zero_is_noop() {
        // 不得 panic；pid 0 必须被忽略（不会误写）。
        attach(0);
    }
}
