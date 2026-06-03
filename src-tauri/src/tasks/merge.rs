use crate::core::{event, git::GitProxy};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use anyhow::{anyhow, Result};
use tracing::info;

pub async fn run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    // Load worktree session
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("no worktree session found for cr {}", cr_id))?;

    // Load CR for project info
    let cr = sqlx::query_as::<_, crate::models::change_request::ChangeRequest>(
        "SELECT * FROM change_requests WHERE id=?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("cr {} not found", cr_id))?;

    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&cr.project_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("project {} not found", cr.project_id))?;

    let git = GitProxy::new(&project.repo_path);

    // Checkout dev branch and merge
    let _ = git.run(&["checkout", &project.branch_dev]).await;

    let (merge_code, _, merge_err) = git
        .run(&[
            "merge",
            "--no-ff",
            &session.branch_name,
            "-m",
            &format!("AutoForge merge: {}", cr_id),
        ])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));

    if merge_code != 0 {
        info!("merge failed ({}): {}", merge_code, merge_err);
        sqlx::query(
            "UPDATE change_requests SET status='merge_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(cr_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE issues SET status='merge_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&cr.issue_id)
        .execute(db)
        .await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_failed".to_string(),
                message: Some(merge_err.chars().take(300).collect()),
            },
        );
        return Err(anyhow!("merge failed for {}: {}", cr_id, merge_err));
    }

    // Remove worktree
    if std::path::Path::new(&session.worktree_path).exists() {
        let _ = git
            .run(&["worktree", "remove", "--force", &session.worktree_path])
            .await;
    }

    // Update CR and issue
    sqlx::query(
        "UPDATE change_requests SET status='merged', updated_at=datetime('now') WHERE id=?",
    )
    .bind(cr_id)
    .execute(db)
    .await?;

    // Find issue to update
    sqlx::query("UPDATE issues SET status='merged', updated_at=datetime('now') WHERE id=?")
        .bind(&cr.issue_id)
        .execute(db)
        .await?;

    sqlx::query(
        "UPDATE worktree_sessions SET status='merged', completed_at=datetime('now') WHERE id=?",
    )
    .bind(&session.id)
    .execute(db)
    .await?;

    sqlx::query(
        "UPDATE preview_environments SET status='terminated', terminated_at=datetime('now') WHERE worktree_session_id=? AND status!='terminated'",
    )
    .bind(&session.id)
    .execute(db)
    .await?;

    event::emit(
        app,
        event::AppEvent::CrMerged {
            cr_id: cr_id.to_string(),
            project_id: cr.project_id.clone(),
        },
    );

    let _ = enqueue(
        db,
        tx,
        "testing",
        &format!("testing:{}", cr_id),
        JobPayload::Testing {
            change_request_id: cr_id.to_string(),
        },
    )
    .await;

    info!("merge completed for cr {}", cr_id);
    Ok(())
}
