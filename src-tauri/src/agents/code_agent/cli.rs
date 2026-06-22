//! 配置驱动的 CLI code agent —— 一个 struct 覆盖 claude / codex / opencode 三种 kind。
//!
//! 设计要点（见 .autoforge/docs/待实现功能/AutoForge-代码Agent可插拔-功能提案.md §4）：
//! - 纯 Rust，零 Tauri 类型，可在非 Tauri 入口复用（CLAUDE.md 铁律 #1）。
//! - 安全不变量由 `run()` 对**所有 kind** 统一施加：传输层禁 remote git
//!   （`GIT_ALLOW_PROTOCOL=""`）、worktree 隔离、进程组隔离。各家 flag 为额外加固。
use super::{CodeAgent, McpInject, RunLimits};
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
        mcp: &[McpInject],
        log: Option<&super::LogSink>,
    ) -> Result<(i32, String, String)> {
        let mut cmd = self.base_cmd(worktree);
        // pull：把「适用于编码 Agent」的 MCP server 注入本 kind 的 CLI。无 server 时为空，命令不变。
        let mcp_cfg = super::mcp_inject::build(&self.profile.kind, mcp);
        for (k, v) in &mcp_cfg.envs {
            cmd.env(k, v);
        }
        // 临时文件（claude 的 mcp-config json）必须活到进程退出，绑定到本作用域。
        let _mcp_temp = mcp_cfg.temp_files;
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
                // MCP 注入（`-c mcp_servers.*` 覆盖）必须在尾部 `-` 之前。
                for a in &mcp_cfg.args {
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
                    // stream-json + verbose：claude 逐条吐 NDJSON 事件（工具调用/文件编辑/
                    // 助手发言/结果），cli 下方解析成可读行实时推前端——否则纯文本 --print
                    // 要等接近结束才一次性出现，无法"实时滚动"。最终报告从转写文本里抽取。
                    .arg("--output-format")
                    .arg("stream-json")
                    .arg("--verbose")
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
                // MCP 注入放最后：含可变参 `--allowedTools mcp__…`，置尾才不会吞掉后续 flag；
                // prompt 走 stdin 不占位置参数，故尾部可变参安全。
                for a in &mcp_cfg.args {
                    cmd.arg(a);
                }
                // prompt 走 stdin：--disallowedTools 是变参，会贪婪吞掉位置参数 prompt。
                true
            }
        };
        // claude（默认/未知 kind）走 stream-json，stdout 需按行解析成可读转写；
        // codex/opencode 是纯文本，stdout 原样透传。
        let claude_stream = !matches!(self.profile.kind.as_str(), "codex" | "opencode");

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

        // stdout_buf：codex/opencode 存原始 stdout；claude 存解析后的**可读转写**
        // （供 extract_report 抽报告 + 落库展示）。line_acc 仅 claude 用，跨读块拼完整 NDJSON 行。
        let mut stdout_buf: Vec<u8> = Vec::new();
        let mut stderr_buf: Vec<u8> = Vec::new();
        let mut line_acc: Vec<u8> = Vec::new();
        // 往实时 sink 发一段日志；通道关闭（前端没在看）时静默忽略。
        let emit = |stream: &'static str, text: String| {
            if !text.is_empty() {
                if let Some(s) = log {
                    let _ = s.send(super::LogChunk { stream, text });
                }
            }
        };
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
                        if claude_stream {
                            // 按行拼接 NDJSON，逐条解析成可读文本后入转写 + 推前端。
                            line_acc.extend_from_slice(&b);
                            while let Some(nl) = line_acc.iter().position(|&c| c == b'\n') {
                                let line: Vec<u8> = line_acc.drain(..=nl).collect();
                                let raw = String::from_utf8_lossy(&line);
                                if let Some(text) = fmt_claude_event(raw.trim_end()) {
                                    stdout_buf.extend_from_slice(text.as_bytes());
                                    stdout_buf.push(b'\n');
                                    emit("stdout", text);
                                }
                            }
                        } else {
                            stdout_buf.extend_from_slice(&b);
                            emit("stdout", String::from_utf8_lossy(&b).to_string());
                        }
                        last_activity = tokio::time::Instant::now();
                    }
                    Some(StreamMsg::Err(b)) => {
                        stderr_buf.extend_from_slice(&b);
                        emit("stderr", String::from_utf8_lossy(&b).to_string());
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
        // 末行无换行符时的残留 NDJSON：补解析一次，避免漏掉最后一个事件（常是 result）。
        if claude_stream && !line_acc.is_empty() {
            let raw = String::from_utf8_lossy(&line_acc);
            if let Some(text) = fmt_claude_event(raw.trim_end()) {
                stdout_buf.extend_from_slice(text.as_bytes());
                stdout_buf.push(b'\n');
                emit("stdout", text);
            }
        }

        if let Some(kind) = timeout_kind {
            // 真杀：SIGKILL 整个进程组（agent + ripgrep/构建子进程），再回收僵尸。
            crate::core::reaper::kill_group(pgid);
            let _ = child.wait().await;
            // 不丢弃已捕获的输出：超时/卡死前的 stdout/stderr 正是调试线索，原样返回
            // （退出码用 124 表超时，调用方按非 0 视为失败），并在 stderr 末尾附说明。
            let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
            let mut stderr = String::from_utf8_lossy(&stderr_buf).to_string();
            stderr.push_str(&format!(
                "\n\n⛔ {} code agent {} timeout（墙钟 {}s / 空闲 {}s）— 已杀进程组回收\n",
                self.profile.kind, kind, limits.wall_secs, limits.idle_secs
            ));
            return Ok((124, stdout, stderr));
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

/// 取首行并限长，用于把工具入参/结果压成一行可读摘要。
fn first_line_clip(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// 把工具调用入参压成一行简述（按工具名挑关键字段）。
fn tool_brief(name: &str, input: Option<&serde_json::Value>) -> String {
    let Some(input) = input else { return String::new() };
    let pick = |k: &str| input.get(k).and_then(|x| x.as_str()).unwrap_or("");
    match name {
        "Bash" => first_line_clip(pick("command"), 160),
        "Edit" | "Write" | "Read" | "NotebookEdit" => pick("file_path").to_string(),
        "Glob" | "Grep" => format!("{} {}", pick("pattern"), pick("path")).trim().to_string(),
        _ => first_line_clip(&input.to_string(), 120),
    }
}

/// 把 tool_result 的 content（可能是字符串或块数组）压成一行反馈摘要。
fn tool_result_brief(content: Option<&serde_json::Value>) -> String {
    let Some(content) = content else { return String::new() };
    if let Some(s) = content.as_str() {
        return first_line_clip(s, 160);
    }
    if let Some(arr) = content.as_array() {
        for block in arr {
            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                return first_line_clip(t, 160);
            }
        }
    }
    String::new()
}

/// 把 claude `--output-format stream-json` 的一行 NDJSON 解析成一段可读文本。
/// 返回 None 表示该事件无需展示（如无文本内容的中间帧）。解析失败也返回 None——
/// 留档/实时只取我们认得的事件，杜绝把裸 JSON 灌进转写。
fn fmt_claude_event(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type").and_then(|t| t.as_str())? {
        "system" => {
            if v.get("subtype").and_then(|s| s.as_str()) == Some("init") {
                let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("");
                Some(format!("● 会话启动 model={model}"))
            } else {
                None
            }
        }
        "assistant" => {
            let content = v.get("message")?.get("content")?.as_array()?;
            let mut out = String::new();
            for block in content {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                            out.push_str(t.trim_end());
                            out.push('\n');
                        }
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        let brief = tool_brief(name, block.get("input"));
                        out.push_str(&format!("🔧 {name} {brief}\n"));
                    }
                    _ => {}
                }
            }
            let out = out.trim_end();
            (!out.is_empty()).then(|| out.to_string())
        }
        "user" => {
            let content = v.get("message")?.get("content")?.as_array()?;
            let mut out = String::new();
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    let brief = tool_result_brief(block.get("content"));
                    if !brief.is_empty() {
                        out.push_str(&format!("  ↳ {brief}\n"));
                    }
                }
            }
            let out = out.trim_end();
            (!out.is_empty()).then(|| out.to_string())
        }
        "result" => {
            let dur = v.get("duration_ms").and_then(|d| d.as_i64()).unwrap_or(0);
            let turns = v.get("num_turns").and_then(|d| d.as_i64()).unwrap_or(0);
            let is_err = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
            Some(format!(
                "{} 完成 · {turns} 轮 · {:.1}s",
                if is_err { "✗" } else { "✓" },
                dur as f64 / 1000.0
            ))
        }
        _ => None,
    }
}
