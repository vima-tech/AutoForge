use serde::{Deserialize, Serialize};

/// `code_agent_skills` 表的一行：注入到 worktree 供编码 agent 消费的技能包。
/// 不含密钥，无需 secrets 加密。`project_id` 为 NULL 表示全局适用于所有项目。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CodeAgentSkillRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub project_id: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// upsert 入参（id 为空表示新建，自动分配 uuid）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertCodeAgentSkill {
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}
