use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub repo_path: String,
    pub branch_dev: String,
    pub branch_main: String,
    pub status: String,
    pub config_yaml: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    /// 软删除时间戳；NULL = 在用，非空 = 已归档（回收站）。
    #[serde(default)]
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProject {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub repo_path: String,
    pub branch_dev: Option<String>,
    pub branch_main: Option<String>,
    pub config_yaml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneProject {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub git_url: String,
    pub target_path: String,
    pub clone_branch: Option<String>,
    pub git_username: Option<String>,
    pub git_password: Option<String>,
    pub branch_dev: Option<String>,
    pub branch_main: Option<String>,
    pub config_yaml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProject {
    pub name: Option<String>,
    pub description: Option<String>,
    pub repo_path: Option<String>,
    pub branch_dev: Option<String>,
    pub branch_main: Option<String>,
    pub status: Option<String>,
    pub config_yaml: Option<String>,
}
