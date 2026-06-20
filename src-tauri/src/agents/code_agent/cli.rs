//! 配置驱动的 CLI code agent —— 一个 struct 覆盖 claude / codex / opencode 三种 kind。
//!
//! 设计要点（见 .autoforge/docs/待实现功能/AutoForge-代码Agent可插拔-功能提案.md §4）：
//! - 纯 Rust，零 Tauri 类型，可在非 Tauri 入口复用（CLAUDE.md 铁律 #1）。
//! - 安全不变量由 `run()` 对**所有 kind** 统一施加：传输层禁 remote git
//!   （`GIT_ALLOW_PROTOCOL=""`）、worktree 隔离、进程组隔离。各家 flag 为额外加固。
use super::CodeAgent;
use anyhow::Result;
use async_trait::async_trait;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

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
        timeout_secs: u64,
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
        if feed_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(prompt.as_bytes()).await?;
                stdin.shutdown().await?; // 关闭 stdin，否则 agent 一直等输入
            }
        } else {
            // 关闭 stdin，避免 agent 误等管道输入。
            drop(child.stdin.take());
        }

        let output =
            tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "{} code agent timed out after {}s",
                        self.profile.kind,
                        timeout_secs
                    )
                })??;

        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
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
