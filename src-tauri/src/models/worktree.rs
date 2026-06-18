use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorktreeSession {
    pub id: String,
    pub change_request_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub status: String,
    pub prompt_snapshot: Option<String>,
    pub iteration_count: i64,
    pub report_content: Option<String>,
    /// 合并前快照的代码 diff（worktree 删除后仍可查看历史改动）。
    pub diff_content: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
