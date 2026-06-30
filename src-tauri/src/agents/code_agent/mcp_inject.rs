//! 把「适用于编码 Agent」的 MCP server 注入到 spawn 出去的 CLI（pull：让编码 agent 在
//! worktree 内实时调用任意 MCP 工具）。这是 push 式代码情报预查之外的第二条路——前者是
//! AutoForge 主动查并注入上下文，后者是 agent 自主按需调用。
//!
//! 三家 CLI 的 MCP 注入机制不同（本模块按 kind 各自生成）：
//! - **claude**（一等公民）：`--mcp-config <临时json>` + `--strict-mcp-config` + `--allowedTools mcp__<name>`
//!   （`--print` 非交互下 MCP 工具必须显式放行，否则被静默拒绝）。
//! - **codex**：`-c mcp_servers.<name>.command=…` 配置覆盖（value 按 TOML 解析；stdio 可行）。
//! - **opencode**：`run` 无逐次注入入口（只有全局 `opencode mcp` 持久配置）→ 暂不支持，留空。
//!
//! 纯 Rust 零 Tauri。env/headers 出库为密文，这里经 `secrets::decrypt` 解密后写入子进程配置。
//! 无 server 时所有 builder 返回空，CLI 命令与改造前完全一致（零回归）。
//!
//! 安全提醒：把任意 MCP 工具暴露给**自主**编码 agent 是信任面扩张——这些工具可能有副作用，
//! 且不受 worktree 的 git/编辑护栏约束。仅 `for_code_agent=1` 且 `enabled=1` 的 server 会注入，
//! 由用户显式勾选；MVP 不在此额外做工具级白名单（沿用各 server 自身的能力边界）。

use crate::models::mcp_server::McpServer;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// 解密后、可直接写进子进程配置的 MCP server 规格（与加密 DB 行解耦）。
#[derive(Debug, Clone)]
pub struct McpInject {
    /// 清洗后的 server 名（用作 mcp 配置键与 `mcp__<name>__tool` 前缀）。
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

/// 名字清洗为 `[A-Za-z0-9_-]`，与 tools/mcp.rs 的工具前缀规则一致。
fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if out.is_empty() { "x".to_string() } else { out }
}

impl McpInject {
    /// 从 DB 行构造（解密 env/headers、解析 args）。
    pub fn from_server(s: &McpServer) -> Self {
        let args: Vec<String> = serde_json::from_str(&s.args_json).unwrap_or_default();
        let env_json = crate::core::secrets::decrypt(&s.env_json).unwrap_or_default();
        let env: BTreeMap<String, String> = serde_json::from_str(&env_json).unwrap_or_default();
        let headers_json = crate::core::secrets::decrypt(&s.headers_json).unwrap_or_default();
        let headers: BTreeMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();
        Self {
            name: sanitize(&s.name),
            transport: s.transport.clone(),
            command: s.command.clone(),
            args,
            env,
            url: s.url.clone(),
            headers,
        }
    }

    fn is_stdio(&self) -> bool {
        self.transport != "http"
    }
}

/// 临时文件守卫：进程结束后删除（claude 的 mcp-config json 用）。
pub struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 一次注入产出：追加到命令行的参数、要设置的环境变量、需保活到进程退出的临时文件。
/// `allowed_tools` 是需放行的工具名（claude `--allowedTools` 的值）——单列出来由 cli.rs
/// 与技能注入的放行项**合并成一次** `--allowedTools`，避免变参被「保留最后一个」覆盖。
#[derive(Default)]
pub struct McpCliConfig {
    pub args: Vec<OsString>,
    pub envs: Vec<(String, String)>,
    pub temp_files: Vec<TempFile>,
    pub allowed_tools: Vec<String>,
}

fn unique_temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "autoforge-mcp-{}-{}-{}.json",
        tag,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

/// claude：写临时 mcpServers json，返回 `--mcp-config <file> --strict-mcp-config --allowedTools mcp__a mcp__b …`。
/// 注意：`--allowedTools` 是可变参，放在 claude 参数序列**最后**，避免吞掉后续 flag。
pub fn for_claude(servers: &[McpInject]) -> McpCliConfig {
    let mut cfg = McpCliConfig::default();
    if servers.is_empty() {
        return cfg;
    }
    let mut map = serde_json::Map::new();
    for s in servers {
        let entry = if s.is_stdio() {
            serde_json::json!({
                "type": "stdio",
                "command": s.command,
                "args": s.args,
                "env": s.env,
            })
        } else {
            serde_json::json!({
                "type": "http",
                "url": s.url,
                "headers": s.headers,
            })
        };
        map.insert(s.name.clone(), entry);
    }
    let doc = serde_json::json!({ "mcpServers": map });
    let path = unique_temp_path("claude");
    if std::fs::write(&path, doc.to_string()).is_err() {
        return McpCliConfig::default(); // 写盘失败 → 不注入，绝不阻断执行。
    }
    cfg.args.push(OsString::from("--mcp-config"));
    cfg.args.push(path.clone().into_os_string());
    cfg.args.push(OsString::from("--strict-mcp-config"));
    // 放行每个 server 的全部工具（mcp__<name> 允许该 server 下所有工具）。
    // 不在此直接发 `--allowedTools`——令牌交给 cli.rs 与技能放行项合并成一次发出。
    for s in servers {
        cfg.allowed_tools.push(format!("mcp__{}", s.name));
    }
    cfg.temp_files.push(TempFile(path));
    cfg
}

/// codex：`-c mcp_servers.<name>.command=… -c mcp_servers.<name>.args=[…] -c mcp_servers.<name>.env={…}`。
/// value 由 codex 按 TOML 解析；直接走 argv 无需 shell 转义。仅注入 stdio（codex 不走 http MCP）。
pub fn for_codex(servers: &[McpInject]) -> McpCliConfig {
    let mut cfg = McpCliConfig::default();
    for s in servers.iter().filter(|s| s.is_stdio()) {
        cfg.args.push(OsString::from("-c"));
        cfg.args.push(OsString::from(format!(
            "mcp_servers.{}.command={}",
            s.name,
            toml_string(&s.command)
        )));
        if !s.args.is_empty() {
            cfg.args.push(OsString::from("-c"));
            cfg.args.push(OsString::from(format!(
                "mcp_servers.{}.args={}",
                s.name,
                toml_array(&s.args)
            )));
        }
        if !s.env.is_empty() {
            cfg.args.push(OsString::from("-c"));
            cfg.args.push(OsString::from(format!(
                "mcp_servers.{}.env={}",
                s.name,
                toml_inline_table(&s.env)
            )));
        }
    }
    cfg
}

/// opencode：暂无逐次注入入口（只有全局 `opencode mcp` 持久配置）→ 不注入，留空。
pub fn for_opencode(_servers: &[McpInject]) -> McpCliConfig {
    McpCliConfig::default()
}

/// 按 kind 分发。
pub fn build(kind: &str, servers: &[McpInject]) -> McpCliConfig {
    match kind {
        "codex" => for_codex(servers),
        "opencode" => for_opencode(servers),
        _ => for_claude(servers),
    }
}

// ── TOML 值编码（codex -c value）──────────────────────────────────────────────
fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
fn toml_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|i| toml_string(i)).collect();
    format!("[{}]", inner.join(","))
}
fn toml_inline_table(map: &BTreeMap<String, String>) -> String {
    let inner: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{}={}", toml_string(k), toml_string(v)))
        .collect();
    format!("{{{}}}", inner.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str, cmd: &str, args: &[&str]) -> McpInject {
        McpInject {
            name: name.to_string(),
            transport: "stdio".to_string(),
            command: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: BTreeMap::new(),
            url: String::new(),
            headers: BTreeMap::new(),
        }
    }

    #[test]
    fn claude_empty_when_no_servers() {
        assert!(for_claude(&[]).args.is_empty());
    }

    #[test]
    fn claude_builds_mcp_config_and_allowlist() {
        let cfg = for_claude(&[stdio("mytool", "node", &["server.js"])]);
        let joined: Vec<String> = cfg.args.iter().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(joined.iter().any(|a| a == "--mcp-config"));
        assert!(joined.iter().any(|a| a == "--strict-mcp-config"));
        // 工具放行令牌单列在 allowed_tools，由 cli.rs 合并发出（不再混在 args 里）。
        assert!(cfg.allowed_tools.iter().any(|a| a == "mcp__mytool"));
        assert!(!joined.iter().any(|a| a == "--allowedTools"));
        assert_eq!(cfg.temp_files.len(), 1);
        // 临时文件确实写了内容。
        let path = &cfg.temp_files[0].0;
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("\"mcpServers\""));
        assert!(content.contains("mytool"));
    }

    #[test]
    fn codex_emits_config_overrides() {
        let mut s = stdio("foo", "codegraph", &["serve", "--mcp"]);
        s.env.insert("TOKEN".into(), "ab\"c".into());
        let cfg = for_codex(&[s]);
        let joined: Vec<String> = cfg.args.iter().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(joined.iter().any(|a| a == "mcp_servers.foo.command=\"codegraph\""));
        assert!(joined.iter().any(|a| a == "mcp_servers.foo.args=[\"serve\",\"--mcp\"]"));
        // 引号被转义。
        assert!(joined.iter().any(|a| a.contains("TOKEN") && a.contains("ab\\\"c")));
    }

    #[test]
    fn opencode_is_noop() {
        assert!(for_opencode(&[stdio("x", "y", &[])]).args.is_empty());
    }
}
