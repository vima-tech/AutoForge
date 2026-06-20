use serde::{Deserialize, Serialize};

/// CR 级测试遥测记录：review_2 合并前自动测试的整体结果。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrTestRun {
    pub id: String,
    pub cr_id: String,
    pub result: String,
    pub summary: String,
    pub run_by: String,
    pub run_at: String,
}
