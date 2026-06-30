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

/// 人工批量绑定（工单组）的相关度预览（preview_batch_bind 返回）。
/// 把「这些需求是否真相关」摊到人眼前，并暴露上限/风险/确认门信号，
/// 取代用 Result 错误通道夹带控制信号的脆弱做法（设计评审 D5）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchBindPreview {
    /// "strong"（实质重叠，鼓励）| "weak"（有重叠但占比低/有冲突）|
    /// "unrelated"（无共享实质文件，真无关）| "insufficient"（含未分析需求，无法判定）。
    pub signal: String,
    /// 共享实质文件占最小成员文件集的比例（0~1）；insufficient 时为 0。
    pub relatedness: f64,
    /// 全员共享的实质文件（已剔除 lib.rs/index.ts 等登记类枢纽）。
    pub shared_files: Vec<String>,
    /// 合并后去重的目标文件总数（blast radius）。
    pub total_files: usize,
    /// 删改冲突提示；无则 None。
    pub conflict_hint: Option<String>,
    /// 缺文件画像（分析失败/无 spec）的成员标题——触发 insufficient 的原因。
    pub missing_analysis: Vec<String>,
    /// 选中数是否超过 MAX_GROUP 上限（超限前端禁用提交）。
    pub over_cap: bool,
    /// 工单组成员数硬上限（= MAX_GROUP），供前端展示。
    pub max_group: usize,
    /// 批次最高风险等级 "low"|"high"（含未分析需求按 high 保守计）；用于提示「将走强模型」。
    pub est_risk: String,
    /// signal ∈ {unrelated, insufficient} → 人工绑定需显式 force 二次确认。
    pub requires_confirm: bool,
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
