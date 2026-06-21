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
    match &msg.payload {
        JobPayload::Analysis { issue_id } => crate::tasks::analysis::run(db, app, issue_id).await,
        JobPayload::Execution {
            change_request_id,
            project_id,
        } => crate::tasks::execution::run(db, tx, app, change_request_id, project_id).await,
        JobPayload::Testing { change_request_id } => {
            crate::tasks::testing::run(db, tx, app, change_request_id).await
        }
        JobPayload::Merge { change_request_id } => {
            crate::tasks::merge::run(db, tx, app, change_request_id).await
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
