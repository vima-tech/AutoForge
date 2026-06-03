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
                }

                // Mark job as running
                let _ = sqlx::query(
                    "UPDATE job_executions SET status='running', started_at=datetime('now'), attempt=attempt+1, updated_at=datetime('now') WHERE id=?"
                )
                .bind(&msg.job_id)
                .execute(&db2)
                .await;

                let result = dispatch_job(&db2, &tx2, &app2, &msg).await;

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
        } => crate::tasks::execution::run(db, app, change_request_id, project_id).await,
        JobPayload::Testing { change_request_id } => {
            crate::tasks::testing::run(db, tx, app, change_request_id).await
        }
        JobPayload::Merge { change_request_id } => {
            crate::tasks::merge::run(db, tx, app, change_request_id).await
        }
    }
}

async fn wait_for_execution_slot(
    db: &Db,
    concurrency: &Arc<ConcurrencyManager>,
    job_id: &str,
    cr_id: &str,
) -> Result<()> {
    loop {
        let status = concurrency.status();
        let (executing,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='executing'")
                .fetch_one(db)
                .await?;
        let (pending_review,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='pending_review_2'")
                .fetch_one(db)
                .await?;

        let paused = pending_review as usize >= status.pause_threshold;
        let full = (executing + pending_review) as usize >= status.max_slots;
        if !paused && !full {
            let result = sqlx::query(
                "UPDATE change_requests SET status='executing', updated_at=datetime('now') WHERE id=? AND status='pending_execution'",
            )
            .bind(cr_id)
            .execute(db)
            .await?;

            if result.rows_affected() > 0 {
                return Ok(());
            }

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

    // INSERT OR IGNORE so re-enqueuing the same key is a no-op
    sqlx::query(
        "INSERT OR IGNORE INTO job_executions (id, idempotency_key, job_type, payload, status) VALUES (?, ?, ?, ?, 'pending')"
    )
    .bind(&id)
    .bind(idempotency_key)
    .bind(job_type)
    .bind(&payload_json)
    .execute(db)
    .await?;

    // Fetch the actual job id (in case the key already existed)
    let row: (String,) = sqlx::query_as("SELECT id FROM job_executions WHERE idempotency_key=?")
        .bind(idempotency_key)
        .fetch_one(db)
        .await?;

    let actual_id = row.0.clone();

    let _ = tx
        .send(JobMsg {
            job_id: actual_id.clone(),
            payload,
        })
        .await;

    Ok(actual_id)
}
