//! Spawned-agent process-group reaping.
//!
//! Code agents (claude / codex / opencode) run in their OWN process group (see
//! [`crate::core::platform::detach_process_group`]) so their signals can't reach
//! our GTK event loop. The flip side: nothing reaps them when they hang, time
//! out, or when AutoForge exits / crashes — they linger as orphans burning CPU
//! (and so do the ripgrep / build / test subprocesses they spawned). This module
//! kills the WHOLE group (agent + every descendant) and tracks live groups so we
//! can sweep them on app exit.
//!
//! Pure Rust, zero Tauri types (CLAUDE.md 铁律 #1).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<HashSet<u32>> {
    static REG: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Record a freshly spawned agent's process-group id (== child pid on unix,
/// since the agent is spawned with `setpgid(0,0)`). `0` is ignored (it would mean
/// "our own group").
pub fn register(pgid: u32) {
    if pgid == 0 {
        return;
    }
    if let Ok(mut g) = registry().lock() {
        g.insert(pgid);
    }
}

/// Stop tracking a group (it exited or was killed).
pub fn unregister(pgid: u32) {
    if let Ok(mut g) = registry().lock() {
        g.remove(&pgid);
    }
}

/// SIGKILL an entire process group by its leader pid — reaps the agent plus every
/// descendant it spawned (ripgrep, bash, tsc, cargo…). Best-effort & idempotent.
/// Never targets group `0` (that would signal our own process group).
pub fn kill_group(pgid: u32) {
    if pgid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        // Negative pid targets the whole group (see kill(2)).
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        // No process-group SIGKILL equivalent; `taskkill /T` kills the tree.
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pgid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    unregister(pgid);
}

/// Kill every still-registered agent group. Called on app exit so a quit/restart
/// never leaves orphaned agents (and their build subprocesses) running.
pub fn kill_all() {
    let pgids: Vec<u32> = registry()
        .lock()
        .map(|g| g.iter().copied().collect())
        .unwrap_or_default();
    for pgid in pgids {
        kill_group(pgid);
    }
}

/// Lower the scheduling priority (raise the *nice* value) of an agent's whole
/// process group so its heavy build/test/search subprocesses yield CPU to the
/// foreground app and the user's machine stays responsive under batch load. Set
/// right after spawn; descendants the agent forks later inherit the nice value.
/// Best-effort; total CPU is unchanged, only its scheduling weight. Unix-only.
pub fn lower_priority(pgid: u32) {
    #[cfg(unix)]
    {
        if pgid == 0 {
            return;
        }
        // +10 ≈ noticeably yields to interactive work without starving the agent.
        unsafe {
            libc::setpriority(libc::PRIO_PGRP, pgid as libc::id_t, 10);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
    }
}

/// Number of logical CPUs (fallback 1). Used to scale the load gate.
pub fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Is the system over `factor × nproc` 1-minute load average? Drives CPU-aware
/// admission backpressure: when true, hold off starting another agent so a batch
/// can't oversubscribe the machine beyond what it can chew. `factor <= 0` disables
/// the gate (always `false`). Linux-only signal (`/proc/loadavg`); elsewhere the
/// gate is a no-op so admission falls back to the slot/pause limits alone.
pub fn system_overloaded(factor: f64) -> bool {
    if factor <= 0.0 {
        return false;
    }
    match load_avg_1m() {
        Some(load1) => load1 > factor * cpu_count() as f64,
        // 信号不可用（非 Linux）→ 闸空转，回退到槽位/暂停阈值。
        None => false,
    }
}

/// 1 分钟系统负载均值（`/proc/loadavg` 第一列）。可观测收敛信号（文档 §6）：核预算调对后
/// 稳态应 `load ≈ nproc`（既不空转也不过载）。`None` = 非 Linux / 不可读。
pub fn load_avg_1m() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/loadavg").ok()?;
        s.split_whitespace().next()?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Total CPU ticks (utime+stime) consumed by every process in group `pgid`.
/// Lets the idle watchdog tell a *busy-but-quiet* agent (running a long build /
/// test that emits nothing) from a *genuinely hung* one (no output AND no CPU) —
/// only the latter should be killed. `None` when the signal is unavailable
/// (non-Linux), so the caller treats idle as output-only there.
pub fn group_cpu_ticks(pgid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if pgid == 0 {
            return None;
        }
        let rd = std::fs::read_dir("/proc").ok()?;
        let mut sum: u64 = 0;
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(pid_s) = name.to_str() else {
                continue;
            };
            if pid_s.parse::<i32>().is_err() {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid_s)) else {
                continue;
            };
            let Some(rparen) = stat.rfind(')') else {
                continue;
            };
            // After the last ')': state ppid pgrp … utime(idx 11) stime(idx 12).
            let toks: Vec<&str> = stat[rparen + 1..].split_whitespace().collect();
            let pgrp = toks.get(2).and_then(|v| v.parse::<i32>().ok());
            if pgrp != Some(pgid as i32) {
                continue;
            }
            let utime = toks.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let stime = toks.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            sum += utime + stime;
        }
        Some(sum)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pgid;
        None
    }
}

/// Best-effort startup sweep: kill agent process groups left over from a PREVIOUS
/// crashed run. The in-memory registry doesn't survive a restart, so we detect
/// leftovers structurally — any live process whose CWD sits inside `base` (the
/// worktrees root) is an agent, or a child it spawned, operating in one of our
/// throwaway worktrees. Linux-only (reads `/proc`); other platforms rely on the
/// on-exit [`kill_all`].
pub fn reap_orphans_under(base: &str) {
    #[cfg(target_os = "linux")]
    {
        if base.is_empty() {
            return;
        }
        let base = std::path::Path::new(base);
        let Ok(rd) = std::fs::read_dir("/proc") else {
            return;
        };
        let mut groups: HashSet<i32> = HashSet::new();
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(pid_s) = name.to_str() else {
                continue;
            };
            // Only numeric pid directories.
            let Ok(pid) = pid_s.parse::<i32>() else {
                continue;
            };
            let cwd = std::fs::read_link(format!("/proc/{}/cwd", pid)).ok();
            let in_base = cwd.as_deref().map(|p| p.starts_with(base)).unwrap_or(false);
            if !in_base {
                continue;
            }
            if let Some(pgid) = read_pgid(pid) {
                if pgid > 0 {
                    groups.insert(pgid);
                }
            }
        }
        if !groups.is_empty() {
            tracing::info!(
                "startup reaper: killing {} orphaned agent group(s) under worktrees",
                groups.len()
            );
        }
        for pgid in groups {
            kill_group(pgid as u32);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = base;
    }
}

/// Parse the process-group id (field `pgrp`) from `/proc/<pid>/stat`. The `comm`
/// field is wrapped in parens and may itself contain spaces/parens, so we split
/// after the LAST ')': the remaining fields are `state ppid pgrp …`.
#[cfg(target_os = "linux")]
fn read_pgid(pid: i32) -> Option<i32> {
    let s = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let rparen = s.rfind(')')?;
    let mut it = s.get(rparen + 1..)?.split_whitespace();
    let _state = it.next()?;
    let _ppid = it.next()?;
    it.next()?.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgid_zero_is_never_targeted() {
        // kill(-0)/setpriority(PRIO_PGRP,0,..) would hit OUR OWN group — these must
        // all be no-ops for pgid 0, never signalling the host process.
        register(0);
        assert!(registry().lock().unwrap().is_empty(), "0 must not be tracked");
        kill_group(0); // must not panic / must not signal us
        lower_priority(0);
    }

    #[test]
    fn load_gate_disabled_when_factor_non_positive() {
        assert!(!system_overloaded(0.0));
        assert!(!system_overloaded(-1.0));
    }

    #[test]
    fn cpu_count_is_at_least_one() {
        assert!(cpu_count() >= 1);
    }

    #[test]
    fn register_unregister_roundtrip() {
        let pgid = 0xBEEF_u32; // not a real pid; never killed in this test
        register(pgid);
        assert!(registry().lock().unwrap().contains(&pgid));
        unregister(pgid);
        assert!(!registry().lock().unwrap().contains(&pgid));
    }
}
