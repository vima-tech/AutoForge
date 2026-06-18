pub mod bulk;
pub mod gateway;
pub mod github;
pub mod proposer;
pub mod ratelimit;
pub mod scanner;
pub mod webhook;

use serde::{Deserialize, Serialize};

/// 入口模式：决定需求落库后是否自动进分析队列。
/// - `Flow`：进 `pending_analysis`，自动入队分析（六个常规通道默认）。
/// - `Triage`：进 `triage` 待整理池，**不**自动分析，等 triage Agent 炼成正经 issue。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntakeMode {
    #[default]
    Flow,
    Triage,
}

impl IntakeMode {
    /// 从可选字符串解析（"triage" → Triage，其余 → Flow）。
    pub fn from_opt(s: Option<&str>) -> Self {
        match s.map(|v| v.trim()) {
            Some("triage") => IntakeMode::Triage,
            _ => IntakeMode::Flow,
        }
    }
    /// 该模式入库后的 issues.status 初值。
    pub fn initial_status(self) -> &'static str {
        match self {
            IntakeMode::Flow => "pending_analysis",
            IntakeMode::Triage => "triage",
        }
    }
}

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
