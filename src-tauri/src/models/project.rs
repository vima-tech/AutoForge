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
