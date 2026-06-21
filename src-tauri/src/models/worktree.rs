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
    /// worktree 从 dev 分叉时的提交 SHA，作为 diff 的固定对比基线。
    /// dev 之后独立前进也不会污染本 CR 的 diff（旧 session 为 NULL 时回退到
    /// 对 target_branch 的实时 diff）。
    pub base_commit: Option<String>,
    /// 合并到 dev 时产生的 squash 提交 SHA，作为「撤销该需求改动」的 revert 对象。
    /// 合并成功后 `git rev-parse HEAD` 回填；旧 session 为 NULL → 撤销入口置灰降级。
    pub merge_commit: Option<String>,
    /// 合并前自动 merge dev 入分支时若冲突，记录冲突文件路径（JSON 数组）。
    pub conflict_files: Option<String>,
    /// 冲突时带冲突标记的快照（供审核页三方视图与 AI 解冲突 agent 输入）。
    pub conflict_diff: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
