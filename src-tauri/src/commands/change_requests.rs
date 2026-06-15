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

    let (repo_path, target_branch) = match cr {
        Some(c) => {
            let proj: Option<(String,)> =
                sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
                    .bind(&c.project_id)
                    .fetch_optional(&state.db)
                    .await
                    .map_err(|e| e.to_string())?;
            (proj.map(|p| p.0).unwrap_or_default(), c.target_branch)
        }
        None => return Ok(String::new()),
    };

    if repo_path.is_empty() {
        return Ok(String::new());
    }

    let _ = repo_path; // kept for the legacy fallback below
    // Diff against the base branch *inside the worktree* — this surfaces the CR's
    // full contribution whether or not it has been committed yet. (Claude Code can't
    // run git, so changes may still be uncommitted; diffing the committed range in the
    // main repo would then return empty and the audit page would hang on "加载中…".)
    let wt_git = GitProxy::new(&session.worktree_path);
    if !target_branch.is_empty() {
        if let Ok((_code, stdout, _stderr)) = wt_git.run(&["diff", &target_branch]).await {
            if !stdout.trim().is_empty() {
                return Ok(stdout);
            }
        }
    }

    // Fallback: base branch unknown or empty diff — show this branch's last commit.
    let diff_arg = format!("{}^1", session.branch_name);
    let result = wt_git.run(&["diff", &diff_arg, &session.branch_name]).await;

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

    // §7: feed the human review-2 outcome into the change class trust state machine
    // (a clean approval extends the streak toward auto-pass; reject/revision resets it).
    let change_class: String = sqlx::query_as::<_, (String,)>(
        "SELECT change_class FROM cr_grades WHERE change_request_id=?",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(|(c,)| c)
    .unwrap_or_else(|| "general".to_string());
    crate::core::gate::record_review_outcome(&state.db, &change_class, decision.decision == "approved").await;

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

/// Remove leftover git worktrees + terminate preview environments for a CR.
/// Best-effort: filesystem/git errors are logged-but-ignored so the DB cleanup
/// (retry reset or row deletion) always proceeds.
async fn cleanup_cr_worktrees(db: &crate::db::Db, cr: &ChangeRequest) {
    let repo_path: Option<(String,)> = sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
        .bind(&cr.project_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    let sessions = sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=?",
    )
    .bind(&cr.id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    if let Some((repo,)) = repo_path {
        if !repo.is_empty() {
            let git = GitProxy::new(&repo);
            for s in &sessions {
                if std::path::Path::new(&s.worktree_path).exists() {
                    let _ = git
                        .run(&["worktree", "remove", "--force", &s.worktree_path])
                        .await;
                }
            }
        }
    }

    // Terminate any live preview environments tied to those sessions.
    let _ = sqlx::query(
        "UPDATE preview_environments SET status='terminated', terminated_at=datetime('now') WHERE worktree_session_id IN (SELECT id FROM worktree_sessions WHERE change_request_id=?) AND status!='terminated'",
    )
    .bind(&cr.id)
    .execute(db)
    .await;
}

/// Recover a stuck change request: re-run code implementation from a clean
/// worktree. Only failed terminal states are retryable so in-flight tasks are
/// never disturbed. This is the closure path for `execution_failed` /
/// `merge_failed` requirements that would otherwise be dead-ended.
#[tauri::command]
pub async fn retry_change_request(
    cr_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    if cr.status != "execution_failed" && cr.status != "merge_failed" {
        return Err(format!(
            "只有执行失败 / 合并失败的需求可以重新执行（当前状态：{}）",
            cr.status
        ));
    }

    // Clear leftover worktrees/previews from the failed attempt before re-running.
    cleanup_cr_worktrees(&state.db, &cr).await;

    sqlx::query(
        "UPDATE change_requests SET status='pending_execution', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE issues SET status='pending_execution', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&cr.issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Unique idempotency key so the retry is never swallowed by INSERT OR IGNORE.
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

    crate::core::event::emit(
        &app,
        crate::core::event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.clone(),
            status: "re-executing".to_string(),
            message: Some("需求已重新进入执行队列".to_string()),
        },
    );

    sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Permanently delete a change request and the whole requirement behind it:
/// worktrees on disk, preview/grade/session/decision rows, the CR row, and the
/// underlying issue + its analysis. This is the "remove abnormal data" closure
/// path so nothing is left orphaned.
#[tauri::command]
pub async fn delete_change_request(
    cr_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let cr = match cr {
        Some(c) => c,
        None => return Ok(()), // already gone — treat delete as idempotent
    };

    // Keep the pending-review counter consistent if we delete a CR awaiting review 2.
    if cr.status == "pending_review_2" {
        state.concurrency.release_pending_review();
    }

    cleanup_cr_worktrees(&state.db, &cr).await;

    // Cascade DB cleanup (preview envs already terminated above; remove the rows).
    let _ = sqlx::query(
        "DELETE FROM preview_environments WHERE worktree_session_id IN (SELECT id FROM worktree_sessions WHERE change_request_id=?)",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM cr_grades WHERE change_request_id=?")
        .bind(&cr_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM worktree_sessions WHERE change_request_id=?")
        .bind(&cr_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM admin_decisions WHERE change_request_id=?")
        .bind(&cr_id)
        .execute(&state.db)
        .await;
    sqlx::query("DELETE FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // Remove the requirement itself so it doesn't linger as an orphan issue.
    let _ = sqlx::query("DELETE FROM issue_analyses WHERE issue_id=?")
        .bind(&cr.issue_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("DELETE FROM issues WHERE id=?")
        .bind(&cr.issue_id)
        .execute(&state.db)
        .await;

    crate::core::event::emit(
        &app,
        crate::core::event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.clone(),
            status: "deleted".to_string(),
            message: Some("需求及其执行数据已删除".to_string()),
        },
    );

    Ok(())
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

    // Innate: capture the human gate decision as project knowledge (fire-and-forget).
    let pid = project_id.to_string();
    let content = format!(
        "审核{stage} 决策：{decision}。管理员建议：{}",
        suggestions.unwrap_or("（无）")
    );
    let trigger = format!("{stage} 阶段同类需求/代码变更的审核判断");
    tokio::spawn(async move {
        crate::knowledge::kb_add(&pid, &content, &trigger).await;
    });
    Ok(())
}
