use crate::core::concurrency::ConcurrencyManager;
use crate::db::Db;
use crate::models::job::JobPayload;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info};
use uuid::Uuid;

pub struct JobMsg {
    pub job_id: String,
    pub payload: JobPayload,
}

pub type JobSender = mpsc::Sender<JobMsg>;

pub fn start(db: Db, app: tauri::AppHandle, concurrency: Arc<ConcurrencyManager>) -> JobSender {
    let (tx, mut rx) = mpsc::channel::<JobMsg>(256);
    let tx_for_worker = tx.clone();

    tauri::async_runtime::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let db2 = db.clone();
            let app2 = app.clone();
            let tx2 = tx_for_worker.clone();
            let concurrency2 = concurrency.clone();
            tauri::async_runtime::spawn(async move {
                let exec_cr_id = if let JobPayload::Execution {
                    change_request_id, ..
                } = &msg.payload
                {
                    Some(change_request_id.clone())
                } else {
                    None
                };
                if let JobPayload::Execution {
                    change_request_id, ..
                } = &msg.payload
                {
                    let wait = wait_for_execution_slot(
                        &db2,
                        &concurrency2,
                        &msg.job_id,
                        change_request_id,
                    )
                    .await;
                    if let Err(e) = wait {
                        error!("job {} failed before dispatch: {}", msg.job_id, e);
                        let _ = sqlx::query(
                            "UPDATE job_executions SET status='failed', last_error=?, updated_at=datetime('now') WHERE id=?"
                        )
                        .bind(e.to_string())
                        .bind(&msg.job_id)
                        .execute(&db2)
                        .await;
                        return;
                    }
                    // Slot granted: account for the now-occupied execution slot.
                    concurrency2.slot_acquired();
                }

                // Mark job as running
                let _ = sqlx::query(
                    "UPDATE job_executions SET status='running', started_at=datetime('now'), attempt=attempt+1, updated_at=datetime('now') WHERE id=?"
                )
                .bind(&msg.job_id)
                .execute(&db2)
                .await;

                let result = dispatch_job(&db2, &tx2, &app2, &msg).await;

                if let Some(cr_id) = &exec_cr_id {
                    // Execution slot is freed once the agent finishes.
                    concurrency2.slot_released();
                    // Count a review slot ONLY if the CR actually parked at
                    // pending_code_review. The auto-merge path (gate downgrade) sends
                    // it straight to pending_merge and never calls review_2, so
                    // counting it here would leak the counter upward forever and
                    // eventually trip the pause threshold.
                    if result.is_ok() {
                        let parked = sqlx::query_as::<_, (String,)>(
                            "SELECT status FROM change_requests WHERE id=?",
                        )
                        .bind(cr_id)
                        .fetch_optional(&db2)
                        .await
                        .ok()
                        .flatten()
                        .map(|(s,)| s == "pending_code_review")
                        .unwrap_or(false);
                        if parked {
                            concurrency2.transition_to_pending_review();
                        }
                    }
                }

                match result {
                    Ok(()) => {
                        let _ = sqlx::query(
                            "UPDATE job_executions SET status='completed', completed_at=datetime('now'), updated_at=datetime('now') WHERE id=?"
                        )
                        .bind(&msg.job_id)
                        .execute(&db2)
                        .await;
                        info!("job {} completed", msg.job_id);
                    }
                    Err(e) => {
                        error!("job {} failed: {}", msg.job_id, e);
                        let _ = sqlx::query(
                            "UPDATE job_executions SET status='failed', last_error=?, updated_at=datetime('now') WHERE id=?"
                        )
                        .bind(e.to_string())
                        .bind(&msg.job_id)
                        .execute(&db2)
                        .await;
                    }
                }
            });
        }
    });

    tx
}

async fn dispatch_job(db: &Db, tx: &JobSender, app: &tauri::AppHandle, msg: &JobMsg) -> Result<()> {
    // M0（EventSink 解耦）：已迁移的任务签名收 `&dyn EventSink` / `&Arc<dyn EventSink>`；
    // `&AppHandle` 自动 unsize 成 `&dyn EventSink` 直接传入，spawn 型任务传共享 Arc sink。
    // 未迁移任务仍收 `app`。待整棵 tasks 迁完，`app` 可整体退役为 sink。
    let sink: std::sync::Arc<dyn crate::core::event::EventSink> =
        std::sync::Arc::new(app.clone());
    match &msg.payload {
        JobPayload::Analysis { issue_id } => {
            crate::tasks::analysis::run(db, app, issue_id).await
        }
        JobPayload::Execution {
            change_request_id,
            project_id,
        } => crate::tasks::execution::run(db, tx, &sink, change_request_id, project_id).await,
        JobPayload::Testing { change_request_id } => {
            crate::tasks::testing::run(db, tx, app, change_request_id).await
        }
        JobPayload::Premerge { change_request_id } => {
            crate::tasks::merge::premerge_run(db, tx, &sink, change_request_id).await
        }
        JobPayload::Merge { change_request_id } => {
            // 已走完 premerge（落了 tested_dev_sha）→ 仅落地（land_run，再校验 dev 漂移）；
            // 否则（开关关 / 旧数据 / 直接 Merge）→ legacy 单锁全流程 run()。
            if crate::tasks::merge::should_land_only(db, change_request_id).await {
                crate::tasks::merge::land_run(db, tx, &sink, change_request_id).await
            } else {
                crate::tasks::merge::run(db, tx, &sink, change_request_id).await
            }
        }
        JobPayload::Revert { change_request_id } => {
            crate::tasks::revert::run(db, app, change_request_id).await
        }
        JobPayload::SecurityAudit { change_request_id } => {
            crate::tasks::security_audit::run(db, tx, app, change_request_id).await
        }
    }
}

async fn wait_for_execution_slot(
    db: &Db,
    concurrency: &Arc<ConcurrencyManager>,
    job_id: &str,
    cr_id: &str,
) -> Result<()> {
    // 负载感知入场阈值（factor×nproc，0=关）。配置很少在等待途中变动，循环外读一次即可。
    let load_factor = crate::commands::system::load_max_load_factor(db).await;
    loop {
        let status = concurrency.status();

        // CPU 背压：批内已有 agent 在跑、且系统 1 分钟负载 > factor×nproc 时，暂缓再起
        // 新 agent，避免把机器压垮（max_slots 之上的动态实测闸）。冷启动（无在跑）不受此
        // 限，以免纯外部负载把流水线完全卡死。
        if load_factor > 0.0
            && status.active_slots >= 1
            && crate::core::reaper::system_overloaded(load_factor)
        {
            let _ = sqlx::query(
                "UPDATE job_executions SET status='waiting', updated_at=datetime('now') WHERE id=?",
            )
            .bind(job_id)
            .execute(db)
            .await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }

        // 核预算背压（文档 §10）：合并门/巡检的验证队列积压——被阻塞等待核令牌的 check 数
        // 超过池容量（即 >1× 过载）——时，暂缓再起新 agent：它们最终的 verify 也会排到这条
        // 队列后面，先放行只会加深积压。与 loadavg 闸同形：仅批内已有 agent 时生效（冷启动
        // 不挡），且是 throttle+重试而非拒绝，队列消化后自动放行，绝不死锁。loadavg（Linux-
        // only）测的是即时系统负载，本闸测的是 AutoForge 自身验证门的排队压力，两者互补。
        if status.active_slots >= 1
            && crate::core::cpu_permits::queue_depth() > crate::core::cpu_permits::total()
        {
            let _ = sqlx::query(
                "UPDATE job_executions SET status='waiting', updated_at=datetime('now') WHERE id=?",
            )
            .bind(job_id)
            .execute(db)
            .await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }

        let (project_id,): (String,) =
            sqlx::query_as("SELECT project_id FROM change_requests WHERE id=?")
                .bind(cr_id)
                .fetch_one(db)
                .await?;

        // Atomic admission: claim the slot only if, at the moment of the write,
        // capacity is still available. Re-checking the counts inside the same
        // UPDATE (executed under SQLite's write lock) closes the check-then-act
        // race where two CRs both observed a free slot and both started.
        let result = sqlx::query(
            "UPDATE change_requests SET status='executing', updated_at=datetime('now')
             WHERE id=? AND status='pending_execution'
               AND (SELECT COUNT(*) FROM change_requests
                    WHERE project_id=? AND status IN ('executing','pending_code_review')) < ?
               AND (SELECT COUNT(*) FROM change_requests
                    WHERE status='pending_code_review') < ?",
        )
        .bind(cr_id)
        .bind(&project_id)
        .bind(status.max_slots as i64)
        .bind(status.pause_threshold as i64)
        .execute(db)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }

        // No slot claimed — distinguish "already running / done" from "still
        // waiting for capacity".
        let (current_status,): (String,) =
            sqlx::query_as("SELECT status FROM change_requests WHERE id=?")
                .bind(cr_id)
                .fetch_one(db)
                .await?;
        if current_status == "executing" {
            return Ok(());
        }
        if current_status != "pending_execution" {
            return Err(anyhow::anyhow!(
                "change request {} is not executable: {}",
                cr_id,
                current_status
            ));
        }

        let _ = sqlx::query(
            "UPDATE job_executions SET status='waiting', updated_at=datetime('now') WHERE id=?",
        )
        .bind(job_id)
        .execute(db)
        .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub async fn enqueue(
    db: &Db,
    tx: &JobSender,
    job_type: &str,
    idempotency_key: &str,
    payload: JobPayload,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let payload_json = serde_json::to_string(&payload)?;

    // INSERT OR IGNORE so re-enqueuing the same key doesn't create a duplicate row.
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO job_executions (id, idempotency_key, job_type, payload, status) VALUES (?, ?, ?, ?, 'pending')"
    )
    .bind(&id)
    .bind(idempotency_key)
    .bind(job_type)
    .bind(&payload_json)
    .execute(db)
    .await?
    .rows_affected()
        > 0;

    // Fetch the actual job id + status (the key may already have existed).
    let (actual_id, status): (String, String) =
        sqlx::query_as("SELECT id, status FROM job_executions WHERE idempotency_key=?")
            .bind(idempotency_key)
            .fetch_one(db)
            .await?;

    // Dispatch ONLY for a fresh job or a previously-failed one being retried. The
    // idempotency key dedups the ROW, not the EXECUTION: an existing pending/waiting/
    // running/completed job is already in flight or done, so re-sending would run it a
    // second time (this send used to be unconditional). Callers that legitimately want
    // a re-run pass a unique key (…:retry:<uuid> / …:restart:<uuid>) → always inserted.
    if inserted || status == "failed" {
        if status == "failed" {
            let _ = sqlx::query(
                "UPDATE job_executions SET status='pending', updated_at=datetime('now') WHERE id=?",
            )
            .bind(&actual_id)
            .execute(db)
            .await;
        }
        let _ = tx
            .send(JobMsg {
                job_id: actual_id.clone(),
                payload,
            })
            .await;
    }

    Ok(actual_id)
}

/// Recover executions orphaned by a previous process exit (crash or quit).
///
/// A queued CR only advances `pending_execution → executing` via the in-memory
/// [`wait_for_execution_slot`] task that [`enqueue`] spawns. That task lives only
/// in this process: if AutoForge restarts while executions are queued or running,
/// the tasks die but the CRs persist at `pending_execution` / `executing` with
/// nothing left to poll for a free slot. Freeing slots afterwards (e.g. a batch
/// merge) then does NOTHING for them — they stall forever, and
/// `retry_change_request` refuses `pending_execution`, so they can't be recovered
/// by hand either.
///
/// Run ONCE at startup, before any driver task exists, so there is never a live
/// task to double up with. Keyed on CR state (not job rows) so it also recovers a
/// CR whose execution job died inside the slot wait (job row `failed`, CR still
/// `pending_execution`):
///   1. roll any half-run `executing` CR back to `pending_execution`, removing its
///      now-stale worktree so the re-dispatched run forks a fresh branch;
///   2. retire the dead execution job rows so stale `waiting`/`running` entries
///      don't linger;
///   3. enqueue a fresh execution job for every `pending_execution` CR, which
///      spawns a new driver task that re-enters the slot gate.
///
/// Returns how many executions were re-enqueued.
pub async fn requeue_orphaned_executions(db: &Db, tx: &JobSender) -> usize {
    // 1) Roll back CRs caught mid-execution; clean the stale worktree.
    let executing: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM change_requests WHERE status='executing'")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    for (cr_id,) in &executing {
        let _ = sqlx::query(
            "UPDATE change_requests SET status='pending_execution', updated_at=datetime('now') WHERE id=? AND status='executing'",
        )
        .bind(cr_id)
        .execute(db)
        .await;
        crate::commands::change_requests::cleanup_cr_worktrees_by_id(db, cr_id).await;
    }

    // 2) Retire dead execution job rows; the fresh enqueue below creates the live one.
    let _ = sqlx::query(
        "UPDATE job_executions SET status='failed', last_error='superseded by restart recovery', updated_at=datetime('now')
         WHERE job_type='execution' AND status IN ('pending','waiting','running')",
    )
    .execute(db)
    .await;

    // 3) Enqueue a fresh execution job for every queued CR (incl. those just rolled back).
    let pending: Vec<(String, String)> =
        sqlx::query_as("SELECT id, project_id FROM change_requests WHERE status='pending_execution'")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    let mut requeued = 0usize;
    for (cr_id, project_id) in pending {
        // Unique key so INSERT OR IGNORE never swallows the recovery enqueue.
        let idem_key = format!("execution:{}:restart:{}", cr_id, Uuid::new_v4());
        if enqueue(
            db,
            tx,
            "execution",
            &idem_key,
            JobPayload::Execution {
                change_request_id: cr_id.clone(),
                project_id,
            },
        )
        .await
        .is_ok()
        {
            requeued += 1;
        }
    }
    if requeued > 0 {
        info!(
            "startup recovery: re-enqueued {} orphaned execution(s)",
            requeued
        );
    }
    requeued
}

/// Recover analyses orphaned either by a previous process exit or by a re-enqueue
/// that silently de-duplicated against an already-`completed` job row.
///
/// An issue leaves `pending_analysis` only when its analysis job runs to completion,
/// and that job is driven by an in-memory task spawned at [`enqueue`]. Two ways an
/// issue gets stuck there with no live task to ever finish it:
///   1. the process exits while analyses are queued/running (the tasks die, the rows
///      persist); or
///   2. a re-analysis was requested under the stable `analysis:<id>` key while a
///      prior job row was already `completed` — [`enqueue`] only re-dispatches a
///      `failed` row, so it skipped the send while the issue had already flipped to
///      `pending_analysis`. (The live call sites now use unique keys; this also
///      sweeps up issues left stranded by the old stable-key behaviour.)
///
/// Run ONCE at startup, before any driver task exists, so there is never a live task
/// to double up with. A fresh UNIQUE-keyed enqueue guarantees re-dispatch regardless
/// of any stale row's state.
///
/// Returns how many analyses were re-enqueued.
pub async fn requeue_orphaned_analyses(db: &Db, tx: &JobSender) -> usize {
    let pending: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM issues WHERE status='pending_analysis'")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    let mut requeued = 0usize;
    for (issue_id,) in pending {
        // Unique key so INSERT OR IGNORE / the failed-only re-dispatch guard never
        // swallows the recovery enqueue.
        let idem_key = format!("analysis:{}:restart:{}", issue_id, Uuid::new_v4());
        if enqueue(
            db,
            tx,
            "analysis",
            &idem_key,
            JobPayload::Analysis {
                issue_id: issue_id.clone(),
            },
        )
        .await
        .is_ok()
        {
            requeued += 1;
        }
    }
    if requeued > 0 {
        info!(
            "startup recovery: re-enqueued {} orphaned analysis(es)",
            requeued
        );
    }
    requeued
}

/// Recover merges orphaned by a previous process exit (crash or quit).
///
/// `review_2`(approved) / the auto-merge gate / 自动解冲突回落 all flip the CR to
/// `pending_merge` and enqueue a Merge job whose live driver task lives only in this
/// process. If AutoForge restarts after the flip but before the merge lands, the CR
/// stays at `pending_merge` with nothing left to drive it — stuck forever (no command
/// re-triggers a `pending_merge` CR by hand).
///
/// Safe to auto-re-enqueue because the Merge job is **idempotent**: it re-derives
/// everything from the worktree session, and `git merge --squash` of an already-landed
/// branch produces no new diff (lands as a no-op rather than double-applying). Worst
/// case for a crash in the exact post-squash/pre-`merged` window is a lost
/// `merge_commit` SHA, which the merge path already tolerates (stores NULL).
///
/// Run ONCE at startup, before any driver task exists. Keyed on CR state, not job rows.
/// `merge_conflict` / `merge_failed` are parked terminal states awaiting human/AI action
/// — deliberately NOT swept here.
pub async fn requeue_orphaned_merges(db: &Db, tx: &JobSender) -> usize {
    // Retire dead premerge/merge job rows so the fresh enqueue below isn't deduped against them.
    let _ = sqlx::query(
        "UPDATE job_executions SET status='failed', last_error='superseded by restart recovery', updated_at=datetime('now')
         WHERE job_type IN ('premerge','merge') AND status IN ('pending','waiting','running')",
    )
    .execute(db)
    .await;

    let mut requeued = 0usize;

    // pending_merge / merge_testing：尚未通过测试门或崩在 premerge 中 → 重排 Premerge
    // （从头重测，幂等安全）。pending_merge 在开关关闭时由 should_land_only 路由回 legacy run。
    let to_premerge: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM change_requests WHERE status IN ('pending_merge','merge_testing')",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();
    for (cr_id,) in to_premerge {
        let idem_key = format!("premerge:{}:restart:{}", cr_id, Uuid::new_v4());
        if enqueue(
            db,
            tx,
            "premerge",
            &idem_key,
            JobPayload::Premerge {
                change_request_id: cr_id.clone(),
            },
        )
        .await
        .is_ok()
        {
            requeued += 1;
        }
    }

    // merge_ready：已测过、只差落地 → 重排 Merge(land)（land_run 再校验 dev 漂移，幂等安全）。
    let to_land: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM change_requests WHERE status='merge_ready'")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    for (cr_id,) in to_land {
        let idem_key = format!("merge:{}:landrestart:{}", cr_id, Uuid::new_v4());
        if enqueue(
            db,
            tx,
            "merge",
            &idem_key,
            JobPayload::Merge {
                change_request_id: cr_id.clone(),
            },
        )
        .await
        .is_ok()
        {
            requeued += 1;
        }
    }

    if requeued > 0 {
        info!("startup recovery: re-enqueued {} orphaned premerge/merge(s)", requeued);
    }
    requeued
}

/// Recover reverts orphaned by a previous process exit (crash or quit).
///
/// `revert_change_request` atomically flips a `merged` CR to `reverting` and enqueues a
/// Revert job. Unlike merge, `git revert` is **NOT idempotent** — re-running after the
/// revert commit already landed would revert the revert (re-applying the change). So we
/// do NOT auto-re-enqueue; we roll the CR back to its prior stable `merged` state and let
/// the operator re-trigger the (manual, user-initiated) revert after confirming dev.
///
/// Returns how many reverts were rolled back.
pub async fn recover_orphaned_reverts(db: &Db) -> usize {
    // Retire any dead revert job rows so they don't linger as phantom in-flight work.
    let _ = sqlx::query(
        "UPDATE job_executions SET status='failed', last_error='superseded by restart recovery', updated_at=datetime('now')
         WHERE job_type='revert' AND status IN ('pending','waiting','running')",
    )
    .execute(db)
    .await;

    let reverting: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, issue_id FROM change_requests WHERE status='reverting'",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut rolled = 0usize;
    for (cr_id, issue_id) in &reverting {
        let _ = sqlx::query(
            "UPDATE change_requests SET status='merged', updated_at=datetime('now') WHERE id=? AND status='reverting'",
        )
        .bind(cr_id)
        .execute(db)
        .await;
        // 合并 CR：全部成员需求一并回滚到 merged。
        let _ = crate::commands::change_requests::set_cr_issues_status(db, cr_id, "merged").await;
        let _ = issue_id; // 兼容旧签名；真源走关联表。
        rolled += 1;
    }
    if rolled > 0 {
        info!(
            "startup recovery: rolled back {} interrupted revert(s) to merged",
            rolled
        );
    }
    rolled
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE job_executions (
                id TEXT PRIMARY KEY, idempotency_key TEXT NOT NULL UNIQUE, job_type TEXT NOT NULL,
                payload TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending', attempt INTEGER NOT NULL DEFAULT 0,
                last_error TEXT, enqueued_at TEXT, started_at TEXT, completed_at TEXT, updated_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        p
    }

    /// Startup recovery: a CR interrupted mid-revert (status `reverting`) must roll back to
    /// its prior stable `merged` state — never be re-run (git revert is not idempotent).
    /// CRs in other states are untouched.
    #[tokio::test]
    async fn recover_reverts_rolls_reverting_back_to_merged() {
        let db = pool().await;
        sqlx::query(
            "CREATE TABLE change_requests (id TEXT PRIMARY KEY, issue_id TEXT, status TEXT, updated_at TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE issues (id TEXT PRIMARY KEY, status TEXT, updated_at TEXT)")
            .execute(&db)
            .await
            .unwrap();
        // 全组联动真源表（迁移 0070）：撤销回滚据此把 CR 的全部成员需求一并置态。
        sqlx::query("CREATE TABLE change_request_issues (change_request_id TEXT, issue_id TEXT, role TEXT, sort_order INTEGER)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO change_requests (id, issue_id, status) VALUES ('cr1','i1','reverting'), ('cr2','i2','merged')")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO issues (id, status) VALUES ('i1','reverting'), ('i2','merged')")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO change_request_issues VALUES ('cr1','i1','primary',0), ('cr2','i2','primary',0)")
            .execute(&db).await.unwrap();

        let rolled = recover_orphaned_reverts(&db).await;
        assert_eq!(rolled, 1, "exactly one reverting CR rolled back");

        let cr1: (String,) = sqlx::query_as("SELECT status FROM change_requests WHERE id='cr1'")
            .fetch_one(&db).await.unwrap();
        assert_eq!(cr1.0, "merged", "interrupted revert returns to merged");
        let i1: (String,) = sqlx::query_as("SELECT status FROM issues WHERE id='i1'")
            .fetch_one(&db).await.unwrap();
        assert_eq!(i1.0, "merged");
        let cr2: (String,) = sqlx::query_as("SELECT status FROM change_requests WHERE id='cr2'")
            .fetch_one(&db).await.unwrap();
        assert_eq!(cr2.0, "merged", "unrelated CR untouched");
    }

    /// M1: the idempotency key dedups the ROW *and* the EXECUTION — re-enqueuing the same
    /// key must NOT dispatch a second JobMsg (the send used to be unconditional). A unique
    /// key (retry/restart path) always dispatches.
    #[tokio::test]
    async fn enqueue_dedups_execution_by_key() {
        let db = pool().await;
        let (tx, mut rx) = mpsc::channel::<JobMsg>(16);
        let p = JobPayload::Merge {
            change_request_id: "cr1".into(),
        };

        enqueue(&db, &tx, "merge", "merge:cr1", p.clone()).await.unwrap();
        enqueue(&db, &tx, "merge", "merge:cr1", p.clone()).await.unwrap();
        assert!(rx.try_recv().is_ok(), "first enqueue should dispatch");
        assert!(
            rx.try_recv().is_err(),
            "same-key re-enqueue must NOT dispatch again"
        );

        enqueue(&db, &tx, "merge", "merge:cr1:retry:x", p).await.unwrap();
        assert!(rx.try_recv().is_ok(), "unique-key enqueue should dispatch");
    }

    /// A failed job re-enqueued under the same key is re-dispatched (and flipped back to
    /// pending) so recovery/retry still works.
    #[tokio::test]
    async fn enqueue_redispatches_failed_key() {
        let db = pool().await;
        let (tx, mut rx) = mpsc::channel::<JobMsg>(16);
        let p = JobPayload::Merge {
            change_request_id: "cr2".into(),
        };
        enqueue(&db, &tx, "merge", "merge:cr2", p.clone()).await.unwrap();
        let _ = rx.try_recv();
        sqlx::query("UPDATE job_executions SET status='failed' WHERE idempotency_key='merge:cr2'")
            .execute(&db)
            .await
            .unwrap();
        enqueue(&db, &tx, "merge", "merge:cr2", p).await.unwrap();
        assert!(rx.try_recv().is_ok(), "failed key should re-dispatch");
        let st: (String,) =
            sqlx::query_as("SELECT status FROM job_executions WHERE idempotency_key='merge:cr2'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(st.0, "pending", "failed job should be reset to pending on re-enqueue");
    }
}
