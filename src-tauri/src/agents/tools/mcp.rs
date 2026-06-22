//! MCP（Model Context Protocol）client：把外部 MCP server 暴露的工具适配成本地
//! [`Tool`] trait，注册进 [`super::ToolRegistry`]，与内置工具走同一条调用 + 安全过滤路径。
//!
//! 约束（CLAUDE.md）：纯 Rust 零 Tauri；MCP 工具结果是不可信外部输入，回灌前由
//! `ToolRegistry::invoke` 统一过 `has_obvious_injection` + 截断（本文件不重复施加）。
//! MVP 支持两种传输：stdio（本地子进程）与 streamable-http（远程）。
//!
//! 生命周期：connect-per-turn——每次为某个 Agent 构建注册表时按需连接其适用的 server，
//! 列出工具，连接句柄由 [`McpTool`] 以 `Arc` 持有，随注册表 drop 而关闭。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;

use super::{Tool, ToolSpec};
use crate::models::mcp_server::McpServer;

type McpService = RunningService<RoleClient, ()>;

/// 一个已建立的 MCP server 连接。被该 server 的所有 [`McpTool`] 以 `Arc` 共享。
pub struct McpConnection {
    service: McpService,
}

impl McpConnection {
    /// 按配置连接 MCP server（stdio 起子进程 / http 连远程）。
    pub async fn connect(server: &McpServer) -> Result<Arc<Self>> {
        let service = match server.transport.as_str() {
            "stdio" => {
                if server.command.trim().is_empty() {
                    return Err(anyhow!("stdio MCP server 未配置 command"));
                }
                // Resolve via PATH so Windows can launch `.cmd`/`.bat` shims
                // (e.g. `npx`, `npm`)—`Command::new` only auto-appends `.exe`.
                let mut cmd = tokio::process::Command::new(crate::core::platform::program(
                    server.command.trim(),
                ));
                let args: Vec<String> = serde_json::from_str(&server.args_json).unwrap_or_default();
                cmd.args(&args);
                let env_json = crate::core::secrets::decrypt(&server.env_json).unwrap_or_default();
                let env: std::collections::BTreeMap<String, String> =
                    serde_json::from_str(&env_json).unwrap_or_default();
                for (k, v) in env {
                    cmd.env(k, v);
                }
                let transport = TokioChildProcess::new(cmd)
                    .map_err(|e| anyhow!("启动 MCP 子进程失败: {}", e))?;
                ().serve(transport)
                    .await
                    .map_err(|e| anyhow!("MCP 握手失败(stdio): {}", e))?
            }
            "http" => {
                if server.url.trim().is_empty() {
                    return Err(anyhow!("http MCP server 未配置 url"));
                }
                let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.trim());
                let headers_json =
                    crate::core::secrets::decrypt(&server.headers_json).unwrap_or_default();
                let headers: std::collections::BTreeMap<String, String> =
                    serde_json::from_str(&headers_json).unwrap_or_default();
                for (k, v) in headers {
                    if let (Ok(name), Ok(val)) = (
                        reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                        reqwest::header::HeaderValue::from_str(&v),
                    ) {
                        config.custom_headers.insert(name, val);
                    }
                }
                let transport =
                    StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);
                ().serve(transport)
                    .await
                    .map_err(|e| anyhow!("MCP 握手失败(http): {}", e))?
            }
            other => return Err(anyhow!("不支持的 MCP 传输类型: {}", other)),
        };
        Ok(Arc::new(Self { service }))
    }

    /// 列出 server 暴露的工具元数据。
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>> {
        self.service
            .list_all_tools()
            .await
            .map_err(|e| anyhow!("列出 MCP 工具失败: {}", e))
    }

    /// 公开调用入口：按远程工具名 + 参数调用，返回拼接后的文本内容。
    /// 供 code_intel 等「push 式」直接消费 MCP 工具的场景使用（绕过 Tool 适配层）。
    /// 注意：结果是不可信外部输入，调用方负责消毒/截断后再回灌上下文。
    pub async fn call_tool(&self, remote_name: &str, args: Value) -> Result<String> {
        self.call(remote_name, args).await
    }

    async fn call(&self, remote_name: &str, args: Value) -> Result<String> {
        let mut param = CallToolRequestParams::new(remote_name.to_string());
        param.arguments = args.as_object().cloned();
        let result = self
            .service
            .call_tool(param)
            .await
            .map_err(|e| anyhow!("MCP 调用失败: {}", e))?;
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        if result.is_error.unwrap_or(false) {
            return Err(anyhow!(
                "MCP 工具 `{}` 返回错误: {}",
                remote_name,
                if text.is_empty() { "(无详情)" } else { &text }
            ));
        }
        Ok(text)
    }
}

/// 适配单个 MCP 工具为本地 [`Tool`]。对外暴露 `mcp__<server>__<tool>` 形式的命名以避免冲突，
/// 实际调用时用 `remote_name`（server 上的原始工具名）。
pub struct McpTool {
    conn: Arc<McpConnection>,
    exposed_name: String,
    remote_name: String,
    description: String,
    parameters: Value,
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.exposed_name.clone(),
            self.description.clone(),
            self.parameters.clone(),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        self.conn.call(&self.remote_name, args).await
    }
}

/// 把工具/服务名清洗成 OpenAI/Anthropic function name 允许的字符集 `[A-Za-z0-9_-]`。
fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "x".to_string()
    } else {
        out
    }
}

/// 连接 server 并返回其全部工具（已适配为 `Arc<dyn Tool>`）。连接句柄由返回的工具共享持有。
pub async fn connect_tools(server: &McpServer) -> Result<Vec<Arc<dyn Tool>>> {
    let conn = McpConnection::connect(server).await?;
    let meta = conn.list_tools().await?;
    let server_slug = sanitize(&server.name);
    let tools = meta
        .into_iter()
        .map(|t| {
            let remote_name = t.name.to_string();
            let exposed_name = format!("mcp__{}__{}", server_slug, sanitize(&remote_name));
            let description = t
                .description
                .map(|d| d.to_string())
                .unwrap_or_else(|| format!("MCP 工具 {}", remote_name));
            let parameters = Value::Object((*t.input_schema).clone());
            Arc::new(McpTool {
                conn: conn.clone(),
                exposed_name,
                remote_name,
                description,
                parameters,
            }) as Arc<dyn Tool>
        })
        .collect();
    Ok(tools)
}

/// 测试连接：连上并返回工具名列表（供设置页“测试连接”按钮）。
pub async fn test_connection(server: &McpServer) -> Result<Vec<String>> {
    let conn = McpConnection::connect(server).await?;
    let meta = conn.list_tools().await?;
    Ok(meta.into_iter().map(|t| t.name.to_string()).collect())
}
