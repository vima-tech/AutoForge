//! 配置驱动的 CLI code agent —— 一个 struct 覆盖 claude / codex / opencode 三种 kind。
//!
//! 设计要点（见 .autoforge/docs/待实现功能/AutoForge-代码Agent可插拔-功能提案.md §4）：
//! - 纯 Rust，零 Tauri 类型，可在非 Tauri 入口复用（CLAUDE.md 铁律 #1）。
//! - 安全不变量由 `run()` 对**所有 kind** 统一施加：传输层禁 remote git
//!   （`GIT_ALLOW_PROTOCOL=""`）、worktree 隔离、进程组隔离。各家 flag 为额外加固。
use super::{CodeAgent, RunLimits};
use anyhow::Result;
use async_trait::async_trait;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// Unregister a tracked agent process group when the run future ends (normal
/// completion, error, or cancellation). Explicit kills happen on timeout / app
/// exit; this guard only drops the bookkeeping entry — it never kills, so a
/// freshly-reused pid is never signalled.
struct GroupGuard(u32);
impl Drop for GroupGuard {
    fn drop(&mut self) {
        crate::core::reaper::unregister(self.0);
    }
}

/// Drained-stream message from the reader tasks to the supervising select loop.
enum StreamMsg {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Eof,
}

/// 解析自 `code_agents` 表的一次性运行档案。
#[derive(Debug, Clone)]
pub struct CliProfile {
    pub kind: String,
    pub program: String,
    pub model: Option<String>,
    pub extra_args: Vec<String>,
}

pub struct CliCodeAgent {
    profile: CliProfile,
}

impl CliCodeAgent {
    pub fn new(profile: CliProfile) -> Self {
        Self { profile }
    }

    /// 硬兜底：未配置任何 code agent 时回落到 claude。
    pub fn claude() -> Self {
        Self::new(CliProfile {
            kind: "claude".into(),
            program: "claude".into(),
            model: None,
            extra_args: Vec::new(),
        })
    }

    fn program(&self) -> std::ffi::OsString {
        crate::core::platform::program(&self.profile.program)
    }

    fn model(&self) -> Option<&str> {
        self.profile
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// 套用所有 kind 共享的安全护栏 + 隔离。
    fn base_cmd(&self, worktree: &str) -> Command {
        let mut cmd = Command::new(self.program());
        // 进程组隔离：子进程信号（claude CLI 的 SIGTRAP 等）不串扰 GTK 事件循环。
        crate::core::platform::detach_process_group(&mut cmd);
        cmd.current_dir(worktree)
            // 传输层禁 remote git：无论 agent 怎么 shell-out，push/fetch/clone 全失败，
            // 本地 commit/build/test 不受影响。这是与具体 CLI 无关的通用护栏。
            .env("GIT_ALLOW_PROTOCOL", "")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

#[async_trait]
impl CodeAgent for CliCodeAgent {
    fn kind(&self) -> &str {
        &self.profile.kind
    }

    async fn run(
        &self,
        worktree: &str,
        prompt: &str,
        limits: RunLimits,
    ) -> Result<(i32, String, String)> {
        let mut cmd = self.base_cmd(worktree);
        // 每个分支构建该 kind 的参数，并返回 prompt 是否走 stdin（true）/ 已作位置参数（false）。
        let feed_stdin = match self.profile.kind.as_str() {
            "codex" => {
                // codex exec 非交互；workspace-write sandbox 可改文件且默认断网
                // （remote git 天然被禁）；--skip-git-repo-check 允许在 worktree 顶层运行。
                cmd.arg("exec")
                    .arg("-C")
                    .arg(worktree)
                    .arg("-s")
                    .arg("workspace-write")
                    .arg("--skip-git-repo-check");
                if let Some(m) = self.model() {
                    cmd.arg("-m").arg(m);
                }
                for a in &self.profile.extra_args {
                    cmd.arg(a);
                }
                // `-` 让 codex 从 stdin 读取指令，避免超长 prompt 撞命令行长度上限。
                cmd.arg("-");
                true
            }
            "opencode" => {
                // opencode run [message]；--dir 指定工作目录。无工具级 git 禁用，
                // 仅靠 base_cmd 的传输层护栏。
                cmd.arg("run").arg("--dir").arg(worktree);
                if let Some(m) = self.model() {
                    cmd.arg("-m").arg(m);
                }
                for a in &self.profile.extra_args {
                    cmd.arg(a);
                }
                cmd.arg(prompt);
                false
            }
            // 默认 = claude。
            _ => {
                cmd.arg("--print")
                    .arg("--permission-mode")
                    .arg("acceptEdits")
                    // 阻断直接 `git ` 形式；配合 base_cmd 的传输层护栏双保险。
                    .arg("--disallowedTools")
                    .arg("Bash(git *)");
                if let Some(m) = self.model() {
                    cmd.arg("--model").arg(m);
                }
                for a in &self.profile.extra_args {
                    cmd.arg(a);
                }
                // prompt 走 stdin：--disallowedTools 是变参，会贪婪吞掉位置参数 prompt。
                true
            }
        };

        let mut child = cmd.spawn()?;

        // The child was spawned with setpgid(0,0), so its pgid == its pid. Track
        // it so timeout / app-exit can SIGKILL the whole group (agent + ripgrep /
        // build / test descendants). `_guard` clears the registry entry when this
        // run ends, however it ends.
        let pgid = child.id().unwrap_or(0);
        crate::core::reaper::register(pgid);
        let _guard = GroupGuard(pgid);
        // 降低整个进程组的调度优先级（nice +10），让批量 agent 的构建/搜索子进程给
        // 前台/UI 让路——总 CPU 不变但机器不卡。子进程 fork 后继承该 nice 值。
        crate::core::reaper::lower_priority(pgid);
        // 纳入 cgroup CPU 预算（Linux，且已启用时）：agent 及其子孙（含自测的 rustc/tsc）
        // 的总 CPU 被内核限到预算内——测试照跑，只是被限速。未启用/非 Linux 时空操作。
        crate::core::cpubudget::attach(pgid);

        if feed_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(prompt.as_bytes()).await?;
                stdin.shutdown().await?; // 关闭 stdin，否则 agent 一直等输入
            }
        } else {
            // 关闭 stdin，避免 agent 误等管道输入。
            drop(child.stdin.take());
        }

        // Drain stdout/stderr incrementally so we can enforce an IDLE timeout (no
        // output for `idle_secs`) on top of the WALL ceiling. `.wait_with_output()`
        // buffers to the end and would hide a hung-but-not-exited agent.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamMsg>(64);
        let mut open_streams = 0u8;
        if let Some(mut out) = child.stdout.take() {
            open_streams += 1;
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match out.read(&mut buf).await {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(StreamMsg::Eof).await;
                            break;
                        }
                        Ok(n) => {
                            if tx.send(StreamMsg::Out(buf[..n].to_vec())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
        if let Some(mut err) = child.stderr.take() {
            open_streams += 1;
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match err.read(&mut buf).await {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(StreamMsg::Eof).await;
                            break;
                        }
                        Ok(n) => {
                            if tx.send(StreamMsg::Err(buf[..n].to_vec())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
        drop(tx); // only the reader tasks hold senders now

        let mut stdout_buf: Vec<u8> = Vec::new();
        let mut stderr_buf: Vec<u8> = Vec::new();
        let wall_deadline =
            tokio::time::Instant::now() + Duration::from_secs(limits.wall_secs.max(1));
        let idle_dur = Duration::from_secs(limits.idle_secs);
        let mut last_activity = tokio::time::Instant::now();
        // CPU 基准：空闲告警时用它判断"安静但在干活"（CPU 仍在涨）还是"真卡死"。
        // ~0.5s CPU 时间（_SC_CLK_TCK 通常 100/s）即算活动，足以区分 hung 与 busy。
        let mut last_cpu = crate::core::reaper::group_cpu_ticks(pgid).unwrap_or(0);
        const CPU_ACTIVE_TICKS: u64 = 50;
        let mut timeout_kind: Option<&'static str> = None;

        while open_streams > 0 {
            let idle_deadline = last_activity + idle_dur;
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(StreamMsg::Out(b)) => {
                        stdout_buf.extend_from_slice(&b);
                        last_activity = tokio::time::Instant::now();
                    }
                    Some(StreamMsg::Err(b)) => {
                        stderr_buf.extend_from_slice(&b);
                        last_activity = tokio::time::Instant::now();
                    }
                    Some(StreamMsg::Eof) => open_streams -= 1,
                    None => break,
                },
                _ = tokio::time::sleep_until(idle_deadline), if limits.idle_secs > 0 => {
                    // 无输出达 idle_secs：再看进程组是否仍在烧 CPU。仍在涨 = 安静地跑长
                    // 构建/测试（claude --print 不流式时尤甚），不杀，重置窗口继续等；
                    // CPU 也不动 = 真卡死，才判 idle 超时。这样杜绝误杀合法长任务。
                    match crate::core::reaper::group_cpu_ticks(pgid) {
                        Some(cur) if cur > last_cpu + CPU_ACTIVE_TICKS => {
                            last_cpu = cur;
                            last_activity = tokio::time::Instant::now();
                        }
                        _ => {
                            timeout_kind = Some("idle");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(wall_deadline) => {
                    timeout_kind = Some("wall");
                    break;
                }
            }
        }

        if let Some(kind) = timeout_kind {
            // 真杀：SIGKILL 整个进程组（agent + ripgrep/构建子进程），再回收僵尸。
            crate::core::reaper::kill_group(pgid);
            let _ = child.wait().await;
            return Err(anyhow::anyhow!(
                "{} code agent {} timeout（墙钟 {}s / 空闲 {}s）— 已杀进程组回收",
                self.profile.kind,
                kind,
                limits.wall_secs,
                limits.idle_secs
            ));
        }

        let status = child.wait().await?;
        let code = status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buf).to_string();
        Ok((code, stdout, stderr))
    }

    async fn check_auth(&self) -> bool {
        match self.profile.kind.as_str() {
            // claude 有缓存的完整登录探测（--version + auth status）。
            "claude" if self.profile.program == "claude" => {
                crate::agents::local_claude::check_auth().await
            }
            // opencode：auth list 退出 0 且列出至少一个已配置 provider。
            "opencode" => {
                let mut cmd = Command::new(self.program());
                crate::core::platform::detach_process_group(&mut cmd);
                match cmd.arg("auth").arg("list").output().await {
                    Ok(o) if o.status.success() => {
                        let text = format!(
                            "{}{}",
                            String::from_utf8_lossy(&o.stdout),
                            String::from_utf8_lossy(&o.stderr)
                        )
                        .to_lowercase();
                        // 无 provider 时通常输出空/"no "/"0 "；有则列出 provider 名。
                        !text.contains("no credentials")
                            && !text.contains("no providers")
                            && text.lines().any(|l| {
                                let l = l.trim();
                                !l.is_empty()
                                    && !l.starts_with('─')
                                    && !l.contains("credentials")
                            })
                    }
                    _ => false,
                }
            }
            // codex 无干净的 auth status；以"已安装"为底线（version 成功）。
            // 真实登录态由用户保证；未装则灰。
            _ => {
                let mut cmd = Command::new(self.program());
                crate::core::platform::detach_process_group(&mut cmd);
                cmd.arg("--version")
                    .output()
                    .await
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            }
        }
    }
}
