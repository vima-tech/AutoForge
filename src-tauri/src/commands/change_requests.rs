use crate::core::git::GitProxy;
use crate::models::change_request::{ChangeRequest, Review1Decision, Review2Decision};
use crate::models::job::JobPayload;
use crate::models::worktree::WorktreeSession;
use crate::state::AppState;
use crate::tasks::runner::enqueue;
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub async fn list_change_requests(
    project_id: Option<String>,
    status: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ChangeRequest>, String> {
    match (project_id, status) {
        (Some(pid), Some(st)) => {
            sqlx::query_as::<_, ChangeRequest>(
                "SELECT * FROM change_requests WHERE project_id=? AND status=? ORDER BY created_at DESC"
            )
            .bind(&pid)
            .bind(&st)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
        }
        (Some(pid), None) => {
            sqlx::query_as::<_, ChangeRequest>(
                "SELECT * FROM change_requests WHERE project_id=? ORDER BY created_at DESC"
            )
            .bind(&pid)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
        }
        (None, Some(st)) => {
            sqlx::query_as::<_, ChangeRequest>(
                "SELECT * FROM change_requests WHERE status=? ORDER BY created_at DESC"
            )
            .bind(&st)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
        }
        (None, None) => {
            sqlx::query_as::<_, ChangeRequest>(
                "SELECT * FROM change_requests ORDER BY created_at DESC"
            )
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
pub async fn get_change_request(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<ChangeRequest>, String> {
    sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_worktree_session(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Option<WorktreeSession>, String> {
    sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_code_diff(cr_id: String, state: State<'_, AppState>) -> Result<String, String> {
    // Get worktree session
    let session: Option<WorktreeSession> = sqlx::query_as(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let session = match session {
        Some(s) => s,
        None => return Ok(String::new()),
    };

    if !std::path::Path::new(&session.worktree_path).exists() {
        return Ok(String::new());
    }

    // Get project repo path
    let cr: Option<ChangeRequest> = sqlx::query_as("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let repo_path = match cr {
        Some(c) => {
            let proj: Option<(String,)> =
                sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
                    .bind(&c.project_id)
                    .fetch_optional(&state.db)
                    .await
                    .map_err(|e| e.to_string())?;
            proj.map(|p| p.0).unwrap_or_default()
        }
        None => return Ok(String::new()),
    };

    if repo_path.is_empty() {
        return Ok(String::new());
    }

    let git = GitProxy::new(&repo_path);
    let diff_arg = format!("{}^1", session.branch_name);
    let result = git.run(&["diff", &diff_arg, &session.branch_name]).await;

    match result {
        Ok((_code, stdout, _stderr)) => Ok(stdout),
        Err(_) => Ok(String::new()),
    }
}

/// Review 1: approve moves to pending_execution and creates a CR + enqueues Execution job
#[tauri::command]
pub async fn review_1(
    issue_id: String,
    decision: Review1Decision,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    if decision.decision == "approved" {
        // Load issue to get project
        let issue =
            sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
                .bind(&issue_id)
                .fetch_one(&state.db)
                .await
                .map_err(|e| e.to_string())?;

        // Load project for default branch
        let project = sqlx::query_as::<_, crate::models::project::Project>(
            "SELECT * FROM projects WHERE id=?",
        )
        .bind(&issue.project_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        // Create CR
        let cr_id = Uuid::new_v4().to_string();
        let admin_id = decision.admin_id.unwrap_or_else(|| "admin".to_string());

        sqlx::query(
            "INSERT INTO change_requests (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch)
             VALUES (?, ?, ?, 'pending_execution', ?, ?, ?)"
        )
        .bind(&cr_id)
        .bind(&issue.project_id)
        .bind(&issue_id)
        .bind(&admin_id)
        .bind(decision.suggestions.as_deref().unwrap_or(""))
        .bind(&project.branch_dev)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        record_admin_decision(
            &state.db,
            &issue.project_id,
            &issue_id,
            Some(&cr_id),
            "review_1",
            "approved",
            &admin_id,
            decision.suggestions.as_deref(),
        )
        .await?;

        // Update issue
        sqlx::query(
            "UPDATE issues SET status='pending_execution', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        // Enqueue execution job
        let idem_key = format!("execution:{}", cr_id);
        let _ = enqueue(
            &state.db,
            &state.job_tx,
            "execution",
            &idem_key,
            JobPayload::Execution {
                change_request_id: cr_id.clone(),
                project_id: issue.project_id.clone(),
            },
        )
        .await;

        sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())
    } else {
        let issue =
            sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
                .bind(&issue_id)
                .fetch_one(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        let admin_id = decision.admin_id.unwrap_or_else(|| "admin".to_string());
        record_admin_decision(
            &state.db,
            &issue.project_id,
            &issue_id,
            None,
            "review_1",
            "rejected",
            &admin_id,
            decision.suggestions.as_deref(),
        )
        .await?;

        // Rejected
        sqlx::query("UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?")
            .bind(&issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        Err("Issue rejected".to_string())
    }
}

/// Review 2: if approved, enqueue Merge job
#[tauri::command]
pub async fn review_2(
    cr_id: String,
    decision: Review2Decision,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    let admin_id = decision.admin_id.unwrap_or_else(|| "admin".to_string());
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    if decision.decision == "approved" {
        record_admin_decision(
            &state.db,
            &cr.project_id,
            &cr.issue_id,
            Some(&cr_id),
            "review_2",
            "approved",
            &admin_id,
            decision.suggestions.as_deref(),
        )
        .await?;

        sqlx::query(
            "UPDATE change_requests SET status='pending_merge', admin_suggestions_2=?, admin_id=?, approved_at=datetime('now'), updated_at=datetime('now') WHERE id=?"
        )
        .bind(decision.suggestions.as_deref().unwrap_or(""))
        .bind(&admin_id)
        .bind(&cr_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        // Enqueue merge job
        let idem_key = format!("merge:{}", cr_id);
        let _ = enqueue(
            &state.db,
            &state.job_tx,
            "merge",
            &idem_key,
            JobPayload::Merge {
                change_request_id: cr_id.clone(),
            },
        )
        .await;

        // Release pending review slot
        state.concurrency.release_pending_review();

        sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())
    } else if decision.decision == "rejected" {
        record_admin_decision(
            &state.db,
            &cr.project_id,
            &cr.issue_id,
            Some(&cr_id),
            "review_2",
            "rejected",
            &admin_id,
            decision.suggestions.as_deref(),
        )
        .await?;

        sqlx::query(
            "UPDATE change_requests SET status='rejected', admin_suggestions_2=?, admin_id=?, updated_at=datetime('now') WHERE id=?"
        )
        .bind(decision.suggestions.as_deref().unwrap_or(""))
        .bind(&admin_id)
        .bind(&cr_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?")
            .bind(&cr.issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE preview_environments SET status='terminated', terminated_at=datetime('now') WHERE worktree_session_id IN (SELECT id FROM worktree_sessions WHERE change_request_id=?) AND status!='terminated'"
        )
        .bind(&cr_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        state.concurrency.release_pending_review();

        crate::core::event::emit(
            &app,
            crate::core::event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.clone(),
                status: "rejected".to_string(),
                message: Some("需求已拒绝，预览环境已终止".to_string()),
            },
        );

        sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())
    } else {
        record_admin_decision(
            &state.db,
            &cr.project_id,
            &cr.issue_id,
            Some(&cr_id),
            "review_2",
            "revision",
            &admin_id,
            decision.suggestions.as_deref(),
        )
        .await?;

        // Send back to execution
        sqlx::query(
            "UPDATE change_requests SET status='pending_execution', admin_suggestions_2=?, admin_id=?, updated_at=datetime('now') WHERE id=?"
        )
        .bind(decision.suggestions.as_deref().unwrap_or(""))
        .bind(&admin_id)
        .bind(&cr_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        // Re-enqueue execution
        let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        let idem_key = format!("execution:{}:retry:{}", cr_id, Uuid::new_v4());
        let _ = enqueue(
            &state.db,
            &state.job_tx,
            "execution",
            &idem_key,
            JobPayload::Execution {
                change_request_id: cr_id.clone(),
                project_id: cr.project_id.clone(),
            },
        )
        .await;

        // emit event for frontend via app handle
        crate::core::event::emit(
            &app,
            crate::core::event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.clone(),
                status: "re-executing".to_string(),
                message: Some("需求已退回重新执行".to_string()),
            },
        );

        sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

async fn record_admin_decision(
    db: &crate::db::Db,
    project_id: &str,
    issue_id: &str,
    change_request_id: Option<&str>,
    stage: &str,
    decision: &str,
    admin_id: &str,
    suggestions: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO admin_decisions
         (id, project_id, issue_id, change_request_id, stage, decision, admin_id, suggestions)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .bind(issue_id)
    .bind(change_request_id)
    .bind(stage)
    .bind(decision)
    .bind(admin_id)
    .bind(suggestions)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
