use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LlmConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    pub ctx_window: String,
    pub temperature: f64,
    pub enabled: bool,
    /// 接口规范：openai | anthropic。决定文本生成路由与工具调用 wire 格式。
    pub api_spec: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLlmConfig {
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    pub ctx_window: Option<String>,
    pub temperature: Option<f64>,
    pub api_spec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateLlmConfig {
    pub name: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub ctx_window: Option<String>,
    pub temperature: Option<f64>,
    pub enabled: Option<bool>,
    pub api_spec: Option<String>,
}
