use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub name_en: String,
    pub role: String,
    pub color: String,
    pub initial: String,
    pub llm_id: Option<String>,
    pub system_prompt: String,
    pub forge_role: Option<String>,
    pub role_type: String,
    pub system_kind: Option<String>,
    pub capabilities_json: String,
    pub max_concurrency: i64,
    pub visible_in_chat: bool,
    pub mentionable: bool,
    pub enabled: bool,
    #[sqlx(default)]
    #[serde(default = "default_prompt_mode")]
    pub prompt_mode: String,
    #[sqlx(default)]
    #[serde(default = "default_true")]
    pub memory_enabled: bool,
    /// 非空 ⇒ 该成员由编码 CLI（claude/codex）只读跑项目仓库作答（会议室答疑），指向
    /// `code_agents.id`；为空 = LLM 后端（用 `llm_id` / 默认 LLM）。旧库迁移前无此列，
    /// `#[sqlx(default)]` 容忍。见 [[迁移 0079]] 与 `agents::code_agent::resolve_by_id`。
    #[sqlx(default)]
    #[serde(default)]
    pub code_agent_id: Option<String>,
    pub created_at: String,
}

fn default_prompt_mode() -> String {
    "builtin".to_string()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgent {
    pub name: String,
    pub name_en: Option<String>,
    pub role: Option<String>,
    pub color: Option<String>,
    pub initial: Option<String>,
    pub llm_id: Option<String>,
    pub system_prompt: Option<String>,
    pub role_type: Option<String>,
    pub system_kind: Option<String>,
    pub capabilities_json: Option<String>,
    pub max_concurrency: Option<i64>,
    pub visible_in_chat: Option<bool>,
    pub mentionable: Option<bool>,
    pub enabled: Option<bool>,
    pub prompt_mode: Option<String>,
    pub memory_enabled: Option<bool>,
    /// 该成员的编码 Agent 后端（`code_agents.id`）；省略/空 = LLM 后端。
    pub code_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub name_en: Option<String>,
    pub role: Option<String>,
    pub color: Option<String>,
    pub llm_id: Option<Option<String>>,
    pub system_prompt: Option<String>,
    pub forge_role: Option<Option<String>>,
    pub role_type: Option<String>,
    pub system_kind: Option<Option<String>>,
    pub capabilities_json: Option<String>,
    pub max_concurrency: Option<i64>,
    pub visible_in_chat: Option<bool>,
    pub mentionable: Option<bool>,
    pub enabled: Option<bool>,
    pub prompt_mode: Option<String>,
    pub memory_enabled: Option<bool>,
    /// 双 Option：外层 None=不改；Some(None)=显式清空（切回 LLM 后端）；Some(Some(id))=绑定
    /// 该编码 Agent。镜像 `llm_id` 的更新语义。
    pub code_agent_id: Option<Option<String>>,
}
