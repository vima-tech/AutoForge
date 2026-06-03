use crate::core::{event, git::GitProxy};
use crate::db::Db;
use crate::state::worktrees_base;
use anyhow::{anyhow, Result};
use tracing::info;
use uuid::Uuid;

pub async fn run(db: &Db, app: &tauri::AppHandle, cr_id: &str, _project_id: &str) -> Result<()> {
    // Load CR
    let cr = sqlx::query_as::<_, crate::models::change_request::ChangeRequest>(
        "SELECT * FROM change_requests WHERE id=?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("change request {} not found", cr_id))?;

    // Load issue
    let issue = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&cr.issue_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow!("issue {} not found", cr.issue_id))?;

    // Load project
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&cr.project_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("project {} not found", cr.project_id))?;

    // Load analysis if available
    let analysis = sqlx::query_as::<_, crate::models::issue::IssueAnalysis>(
        "SELECT * FROM issue_analyses WHERE issue_id=?",
    )
    .bind(&issue.id)
    .fetch_optional(db)
    .await?;

    let analysis_summary = analysis.map(|a| a.analysis_summary).unwrap_or_default();

    let (previous_iterations,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM worktree_sessions WHERE change_request_id=?")
            .bind(cr_id)
            .fetch_one(db)
            .await?;
    let iteration = previous_iterations + 1;

    // Iteration soft limit (design §10.4): warn at >= 3 rounds, never force-stop.
    const ITERATION_SOFT_LIMIT: i64 = 3;
    if iteration >= ITERATION_SOFT_LIMIT {
        event::emit(
            app,
            event::AppEvent::IterationWarning {
                cr_id: cr_id.to_string(),
                iteration,
                soft_limit: ITERATION_SOFT_LIMIT,
            },
        );
    }

    // Create worktree branch and path
    let branch_name = format!("autoforge/{}-i{}", cr_id, iteration);
    let worktree_path = format!("{}/{}-i{}", worktrees_base(), cr_id, iteration);

    let git = GitProxy::new(&project.repo_path);

    // Ensure dev branch exists
    let _ = git.run(&["checkout", &project.branch_dev]).await;

    // Create worktree
    tokio::fs::create_dir_all(&worktrees_base()).await.ok();
    let (wt_code, _, wt_err) = git
        .run(&[
            "worktree",
            "add",
            "-b",
            &branch_name,
            &worktree_path,
            &project.branch_dev,
        ])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));

    if wt_code != 0 {
        info!(
            "worktree add failed ({}), aborting execution: {}",
            wt_code, wt_err
        );
        sqlx::query(
            "UPDATE change_requests SET status='execution_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(cr_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE issues SET status='execution_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&issue.id)
        .execute(db)
        .await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "execution_failed".to_string(),
                message: Some(wt_err.chars().take(300).collect()),
            },
        );
        return Err(anyhow!("worktree add failed for {}: {}", cr_id, wt_err));
    }

    // Create WorktreeSession record
    let session_id = Uuid::new_v4().to_string();
    let prompt = crate::agents::code_agent::build_prompt(
        &issue.title,
        &issue.description,
        &analysis_summary,
        cr.admin_suggestions_2
            .as_deref()
            .or(cr.admin_suggestions_1.as_deref()),
        iteration as u32,
        &project.repo_path,
        project.config_yaml.as_deref(),
    );

    sqlx::query(
        "INSERT INTO worktree_sessions
         (id, change_request_id, worktree_path, branch_name, status, prompt_snapshot, iteration_count, started_at)
         VALUES (?, ?, ?, ?, 'running', ?, ?, datetime('now'))"
    )
    .bind(&session_id)
    .bind(cr_id)
    .bind(&worktree_path)
    .bind(&branch_name)
    .bind(&prompt)
    .bind(iteration)
    .execute(db)
    .await?;

    let preview_id = Uuid::new_v4().to_string();
    let preview_url = format!("file://{}", worktree_path);
    sqlx::query(
        "INSERT INTO preview_environments
         (id, project_id, env_type, worktree_session_id, preview_url, db_snapshot_name, status, ready_at)
         VALUES (?, ?, 'worktree', ?, ?, ?, 'ready', datetime('now'))",
    )
    .bind(&preview_id)
    .bind(&project.id)
    .bind(&session_id)
    .bind(&preview_url)
    .bind(format!("preview_{}_{}_i{}", project.slug, cr_id, iteration))
    .execute(db)
    .await?;

    event::emit(
        app,
        event::AppEvent::PreviewUpdate {
            cr_id: cr_id.to_string(),
            preview_id: preview_id.clone(),
            status: "ready".to_string(),
            preview_url: Some(preview_url),
        },
    );

    // Update CR status
    sqlx::query(
        "UPDATE change_requests SET status='executing', updated_at=datetime('now') WHERE id=?",
    )
    .bind(cr_id)
    .execute(db)
    .await?;
    sqlx::query("UPDATE issues SET status='executing', updated_at=datetime('now') WHERE id=?")
        .bind(&issue.id)
        .execute(db)
        .await?;

    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "running".to_string(),
            message: Some("开始执行代码实现".to_string()),
        },
    );

    // Run claude code agent
    let timeout_secs = 1800; // 30 minutes
    let (exit_code, stdout, _stderr) =
        crate::agents::code_agent::run(&worktree_path, &prompt, timeout_secs)
            .await
            .unwrap_or_else(|e| (-1, format!("Agent error: {}", e), String::new()));

    let report = crate::agents::code_agent::extract_report(&stdout).to_string();

    // Update session
    let new_status = if exit_code == 0 {
        "completed"
    } else {
        "failed"
    };
    sqlx::query(
        "UPDATE worktree_sessions SET status=?, report_content=?, completed_at=datetime('now') WHERE id=?"
    )
    .bind(new_status)
    .bind(&report)
    .bind(&session_id)
    .execute(db)
    .await?;

    if exit_code != 0 {
        sqlx::query(
            "UPDATE change_requests SET status='execution_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(cr_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE issues SET status='execution_failed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&issue.id)
        .execute(db)
        .await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "execution_failed".to_string(),
                message: Some(report.chars().take(200).collect()),
            },
        );
        return Err(anyhow!("code agent failed for {}", cr_id));
    }

    // Update CR to pending_review_2
    sqlx::query(
        "UPDATE change_requests SET status='pending_review_2', updated_at=datetime('now') WHERE id=?"
    )
    .bind(cr_id)
    .execute(db)
    .await?;

    sqlx::query(
        "UPDATE issues SET status='pending_review_2', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&issue.id)
    .execute(db)
    .await?;

    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "completed".to_string(),
            message: Some(report.chars().take(200).collect()),
        },
    );

    event::emit(
        app,
        event::AppEvent::ReviewNeeded {
            cr_id: cr_id.to_string(),
            issue_title: issue.title,
            stage: 2,
        },
    );

    info!("execution task completed for cr {}", cr_id);
    Ok(())
}
