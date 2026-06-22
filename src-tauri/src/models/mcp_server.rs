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
    /// 是否适用于「编码 Agent」：true 时该 server 参与代码实现前的 push 式代码情报预查
    /// （按 capability_map_json 调用并注入 prompt）。与 agent_ids_json（角色 Agent，pull）正交，可同时成立。
    #[serde(default)]
    pub for_code_agent: bool,
    /// 能力→工具映射（仅 for_code_agent 的 push 流用）。形如
    /// `{"locate_symbol":{"tool":"...","args":{"query":"$SYMBOL","projectPath":"$REPO"}}}`，
    /// args 内 `$SYMBOL` / `$REPO` 为占位符。留空则按工具命名约定自动发现。
    #[serde(default)]
    pub capability_map_json: String,
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
    pub for_code_agent: Option<bool>,
    pub capability_map_json: Option<String>,
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
    pub for_code_agent: Option<bool>,
    pub capability_map_json: Option<String>,
    pub enabled: Option<bool>,
}
