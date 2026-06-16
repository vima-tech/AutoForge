use serde::{Deserialize, Serialize};

/// MCP server 配置。`agent_ids_json` 是适用 Agent id 的 JSON 数组（适用范围由 MCP 侧勾选）。
/// stdio: command/args_json/env_json；http: url/headers_json。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    /// 传输方式：stdio | http
    pub transport: String,
    pub command: String,
    pub args_json: String,
    pub env_json: String,
    pub url: String,
    pub headers_json: String,
    pub agent_ids_json: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMcpServer {
    pub name: String,
    pub transport: Option<String>,
    pub command: Option<String>,
    pub args_json: Option<String>,
    pub env_json: Option<String>,
    pub url: Option<String>,
    pub headers_json: Option<String>,
    pub agent_ids_json: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMcpServer {
    pub name: Option<String>,
    pub transport: Option<String>,
    pub command: Option<String>,
    pub args_json: Option<String>,
    pub env_json: Option<String>,
    pub url: Option<String>,
    pub headers_json: Option<String>,
    pub agent_ids_json: Option<String>,
    pub enabled: Option<bool>,
}
