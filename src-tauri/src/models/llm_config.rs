use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LlmConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    /// 上下文窗口（展示用参考值，如 "200K"）。**后端自动推断**（查表 + 接口探测），
    /// 不再由用户手填；拿不到时为「未知」。见 `core::ctx_window`。
    pub ctx_window: String,
    pub temperature: f64,
    pub enabled: bool,
    /// 接口规范：openai | anthropic。决定文本生成路由与工具调用 wire 格式。
    pub api_spec: String,
    /// 是否支持多模态（图片识别）。**后端按模型名自动推断**（见 `core::vision`），
    /// 不再由用户手动开关；true 时绑定该 LLM 的角色 Agent 可在会议室识别图片附件。
    pub supports_vision: bool,
    /// 全局默认 LLM：角色 Agent 未显式绑定 LLM 时回落到此配置（命令层保证至多一个）。
    /// 复用 code_agents.is_default 同款范式。旧库迁移前无此列，`#[sqlx(default)]` 容忍。
    #[sqlx(default)]
    #[serde(default)]
    pub is_default: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLlmConfig {
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    pub temperature: Option<f64>,
    pub api_spec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateLlmConfig {
    pub name: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub temperature: Option<f64>,
    pub enabled: Option<bool>,
    pub api_spec: Option<String>,
}
