pub mod bulk;
pub mod gateway;
pub mod github;
pub mod scanner;
pub mod webhook;

use serde::{Deserialize, Serialize};

/// 统一需求入口载荷，所有渠道均转换为此结构后进入网关
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakePayload {
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub source_type: String,
    pub source_ref: Option<String>,
}
