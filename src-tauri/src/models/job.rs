use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct JobExecution {
    pub id: String,
    pub idempotency_key: String,
    pub job_type: String,
    pub payload: String,
    pub status: String,
    pub attempt: i64,
    pub last_error: Option<String>,
    pub enqueued_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum JobPayload {
    Analysis {
        issue_id: String,
    },
    Execution {
        change_request_id: String,
        project_id: String,
    },
    Testing {
        change_request_id: String,
    },
    /// 合并前阶段（Phase1 dev-sync + 测试门 + 安全门），不持 merge_lock，多 CR 可并行
    /// （受 core::cpu_permits 核预算节流）。通过后置 merge_ready 并入队 Merge(land)。
    Premerge {
        change_request_id: String,
    },
    /// 落地阶段：持 merge_lock 串行，再校验 dev 是否前进后 land 到 dev。
    /// 当 CR 未经 premerge（开关关 / 旧数据）时，merge::land_run 路由回 legacy 全流程。
    Merge {
        change_request_id: String,
    },
    /// 撤销一个已合并 CR 的改动：在 dev 上 `git revert` 其 squash 提交。
    Revert {
        change_request_id: String,
    },
    SecurityAudit {
        change_request_id: String,
    },
}
