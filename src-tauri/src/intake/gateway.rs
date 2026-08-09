use super::{IntakeMode, IntakePayload};
use crate::core::{event, security};
use crate::db::Db;
use crate::models::issue::Issue;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use uuid::Uuid;

/// 统一需求接收网关：去重、安全检查、入库、入队分析。
/// 薄包装，丢弃「是否新建」标志，兼容既有调用方。
pub async fn receive(
    db: &Db,
    job_tx: &JobSender,
    sink: &dyn crate::core::event::EventSink,
    payload: IntakePayload,
    mode: IntakeMode,
) -> Result<Issue, String> {
    receive_with_status(db, job_tx, sink, payload, mode)
        .await
        .map(|(issue, _created)| issue)
}

/// 同 [`receive`]，但额外返回 `created`：仅当**真正 INSERT 了一条全新条目**时为 `true`。
/// 命中既有活跃条目（去重）或返回既有已拒绝条目（Triage 不复活）时为 `false`。
/// 自喂料据此**只对新条目计预算/计入待整理**，避免重复条目占满 `max_per_run`、饿死 proposer。
pub async fn receive_with_status(
    db: &Db,
    job_tx: &JobSender,
    sink: &dyn crate::core::event::EventSink,
    payload: IntakePayload,
    mode: IntakeMode,
) -> Result<(Issue, bool), String> {
    if security::has_obvious_injection(&payload.title) {
        return Err("标题包含可疑内容，提交被拒绝".to_string());
    }
    let desc = payload.description.as_deref().unwrap_or("");
    if security::has_obvious_injection(desc) {
        return Err("描述包含可疑内容，提交被拒绝".to_string());
    }

    let title = security::safe_truncate(&payload.title, 200);
    let description = security::safe_truncate(desc, 4000);

    let fp = security::fingerprint(&title, &description);

    let category = payload.category.unwrap_or_else(|| "Feature".to_string());
    let severity = payload.severity.unwrap_or_else(|| "medium".to_string());
    // triage 模式落「待整理池」(status='triage')，保留原始文本，且不自动入队分析。
    let status = mode.initial_status();
    let raw_capture = (mode == IntakeMode::Triage).then(|| description.clone());

    // 去重：同指纹的**活跃**需求直接返回（幂等，避免重复条目）。
    // 但若同指纹的旧需求已被「拒绝」(rejected) —— 它已从活跃池归档、对用户不可见 ——
    // 此时再次提交是用户明确想重新提出该需求，必须「复活」而非静默返回那条隐藏的死记录，
    // 否则用户会看到「提交成功」却在总账里找不到任何新需求（沉淀为需求看似无效）。
    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT id, status FROM issues WHERE fingerprint=? AND project_id=?")
            .bind(&fp)
            .bind(&payload.project_id)
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())?;

    if let Some((dup_id, dup_status)) = existing {
        if dup_status != "rejected" {
            // 活跃需求：真正的重复，返回已存在的条目（created=false，不计预算）。
            let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
                .bind(&dup_id)
                .fetch_one(db)
                .await
                .map_err(|e| e.to_string())?;
            return Ok((issue, false));
        }
        // 已拒绝且为 Triage（自喂料）来源：人工已明确否决过该指纹，**不复活**——
        // 否则扫描器每轮都会把被拒条目重新刷回待整理池（噪音复发）。直接返回既有
        // 已拒绝条目（created=false），由自喂料跳过、不计预算、不计入待整理。
        if mode == IntakeMode::Triage {
            let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
                .bind(&dup_id)
                .fetch_one(db)
                .await
                .map_err(|e| e.to_string())?;
            return Ok((issue, false));
        }
        // 已拒绝且为 Flow（用户明确重新提出）：复活为本次提交的初始状态，刷新内容与时间戳，重新进入活跃池。
        sqlx::query(
            "UPDATE issues
             SET status=?, title=?, description=?, category=?, severity=?,
                 source_type=?, source_ref=?, raw_capture=?, updated_at=datetime('now')
             WHERE id=?",
        )
        .bind(status)
        .bind(&title)
        .bind(&description)
        .bind(&category)
        .bind(&severity)
        .bind(&payload.source_type)
        .bind(&payload.source_ref)
        .bind(&raw_capture)
        .bind(&dup_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;

        if mode == IntakeMode::Flow {
            let _ = enqueue(
                db,
                job_tx,
                "analysis",
                &format!("analysis:{}", dup_id),
                JobPayload::Analysis { issue_id: dup_id.clone() },
            )
            .await;
        }

        let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&dup_id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string())?;

        event::emit(
            sink,
            event::AppEvent::IssueCreated {
                issue_id: issue.id.clone(),
                project_id: issue.project_id.clone(),
            },
        );

        // 复活既有条目（非全新行）→ created=false。
        return Ok((issue, false));
    }

    // 近似去重（仅机器生成的 Triage 来源，绝不作用于用户的 Flow 提交——用户的明确需求
    // 不能被模糊匹配静默吞掉）：精确指纹未命中时，proposer/scanner 常换个措辞重提同一问题，
    // 精确哈希抓不住。这里对同项目内**活跃**（非终态）需求的标题做字符 bigram Jaccard 相似度，
    // 超过保守阈值即判为重复，返回既有条目（created=false → 不计预算、不淹没队列）。
    if mode == IntakeMode::Triage {
        const SIM_THRESHOLD: f64 = 0.72;
        let incoming = security::title_shingles(&title);
        if !incoming.is_empty() {
            let candidates: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, title FROM issues
                 WHERE project_id=?
                   AND status NOT IN ('merged','rejected','reverted','deferred','no_change_needed')",
            )
            .bind(&payload.project_id)
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
            for (cid, ctitle) in candidates {
                if security::jaccard(&incoming, &security::title_shingles(&ctitle)) >= SIM_THRESHOLD {
                    let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
                        .bind(&cid)
                        .fetch_one(db)
                        .await
                        .map_err(|e| e.to_string())?;
                    return Ok((issue, false));
                }
            }
        }
    }

    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO issues
         (id, project_id, source_type, title, description, category, severity, status, fingerprint, source_ref, raw_capture)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.project_id)
    .bind(&payload.source_type)
    .bind(&title)
    .bind(&description)
    .bind(&category)
    .bind(&severity)
    .bind(status)
    .bind(&fp)
    .bind(&payload.source_ref)
    .bind(&raw_capture)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 仅 Flow 模式自动入队分析；triage 待整理池等人/triage Agent 处理后再转 Flow。
    if mode == IntakeMode::Flow {
        let idem_key = format!("analysis:{}", id);
        let _ = enqueue(
            db,
            job_tx,
            "analysis",
            &idem_key,
            JobPayload::Analysis { issue_id: id.clone() },
        )
        .await;
    }

    let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    // 上下文基质登记（基质设计 §3.2）：新需求一入库即投影为 ContextItem，让后续任意
    // 环节（编码/审核/会议室）可按需取用其正文。best-effort，登记失败不阻断入库。
    {
        use crate::core::context;
        let cref = format!("issue:{}", issue.id);
        let _ = context::register(
            db,
            context::NewContextItem {
                project_id: &issue.project_id,
                source_kind: context::source_kind::ISSUE,
                source_id: &issue.id,
                title: &issue.title,
                origin_stage: "requirement",
                origin_actor: &issue.source_type,
                content_ref: &cref,
                size_hint: issue.description.len() as i64,
                trust: context::trust::TRUSTED,
                labels: "[]",
            },
        )
        .await;
    }

    event::emit(
        sink,
        event::AppEvent::IssueCreated {
            issue_id: issue.id.clone(),
            project_id: issue.project_id.clone(),
        },
    );

    // 全新 INSERT → created=true（自喂料据此计预算/计入待整理）。
    Ok((issue, true))
}
