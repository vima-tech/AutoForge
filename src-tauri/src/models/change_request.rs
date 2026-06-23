use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChangeRequest {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub status: String,
    pub admin_id: Option<String>,
    pub approved_at: Option<String>,
    pub admin_suggestions_1: Option<String>,
    pub admin_suggestions_2: Option<String>,
    pub merge_commit_message: Option<String>,
    pub target_branch: String,
    /// 会议室「立即编码」express CR 携带的讨论上下文（对话快照 + 项目上下文文档）；
    /// 注入编码工单的「需求来源」段。普通流水线 CR 为 NULL。
    pub work_context: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 同文件多需求合并候选组（list_merge_candidates 返回；前端在需求审核闸呈现）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCandidate {
    /// 组内需求 id（建议把它们合并成一个 CR）。
    pub issue_ids: Vec<String>,
    /// 与 issue_ids 同序的需求标题，便于前端直接展示。
    pub titles: Vec<String>,
    /// 全体成员共享的实质文件（已剔除 lib.rs/index.ts 等登记类枢纽文件）。
    pub shared_files: Vec<String>,
    /// 合并后去重的目标文件总数（blast radius）。
    pub total_files: usize,
    /// "strong"（自动建议）| "weak"（blast 过大或有冲突，弱化展示）。
    pub strength: String,
    /// 检出的潜在冲突提示；无则 None。
    pub conflict_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review1Decision {
    pub decision: String,
    pub suggestions: Option<String>,
    pub admin_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review2Decision {
    pub decision: String,
    pub suggestions: Option<String>,
    pub admin_id: Option<String>,
    /// 人工填写的合并提交信息；空时合并任务回退默认模板。
    pub commit_message: Option<String>,
}
