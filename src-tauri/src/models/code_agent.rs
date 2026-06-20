use serde::{Deserialize, Serialize};

/// `code_agents` 表的一行：一个可选的代码实现 agent 配置。
/// `kind` 决定底层适配逻辑（claude / codex / opencode），`program`/`model`/`extra_args_json`
/// 是用户可调的运行参数。不含密钥，无需 secrets 加密。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CodeAgentRow {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub program: String,
    pub model: Option<String>,
    pub extra_args_json: String,
    pub enabled: bool,
    pub is_default: bool,
    pub created_at: String,
}

/// upsert 入参（id 为空表示新建，自动分配 uuid）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertCodeAgent {
    pub id: Option<String>,
    pub kind: String,
    pub label: String,
    pub program: String,
    pub model: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}
