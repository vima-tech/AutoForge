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

/// Compute the CR's full code diff by running git **inside its worktree**.
///
/// Returns `None` when the worktree directory no longer exists (e.g. it was torn
/// down after a successful merge) — callers should then fall back to the diff
/// snapshot persisted on the session. Returns `Some(diff)` otherwise, where an
/// empty string is an authoritative "this branch changed nothing" answer.
pub async fn compute_worktree_diff(
    worktree_path: &str,
    branch_name: &str,
    target_branch: &str,
    base_commit: Option<&str>,
) -> Option<String> {
    if !std::path::Path::new(worktree_path).exists() {
        return None;
    }

    // Diff against the fork point *inside the worktree* — this surfaces the CR's
    // full contribution whether or not it has been committed yet. (Claude Code can't
    // run git, so changes may still be uncommitted; diffing the committed range in the
    // main repo would then return empty and the audit page would hang on "加载中…".)
    let wt_git = GitProxy::new(worktree_path);

    // Scope the diff to the CR's own contribution by diffing against the *fork
    // point* rather than the moving `target_branch` tip (any commit landing on dev
    // after the fork would otherwise leak, inverted, into this CR's diff).
    //
    // Prefer `merge-base(dev, branch)` over the recorded `base_commit` SHA: the two
    // agree (= the fork commit) while the branch hasn't integrated dev, but once
    // Phase 1 merges dev INTO the branch (so land is conflict-free), merge-base
    // advances to dev's tip — making `git diff merge-base` correctly exclude dev's
    // own changes, whereas the static `base_commit` would re-surface them. merge-base
    // is thus correct in both states; `base_commit` is the fallback when it (or
    // target_branch) is unavailable. Empty result is authoritative: branch added nothing.
    let merge_base = if !target_branch.is_empty() {
        match wt_git.run(&["merge-base", target_branch, branch_name]).await {
            Ok((0, out, _)) if !out.trim().is_empty() => Some(out.trim().to_string()),
            _ => None,
        }
    } else {
        None
    };
    let base = merge_base
        .or_else(|| base_commit.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| target_branch.to_string());
    if !base.is_empty() {
        // Diffing the working tree against the fork point captures the CR's full
        // contribution — committed or not. An *empty* result is authoritative:
        // the branch added nothing (e.g. a `no_change_needed` CR where the agent
        // made no commit), so return it verbatim. Falling through to the
        // `branch^1..branch` fallback here would instead surface the base
        // commit's own unrelated changes (showing spurious `/dev/null` additions
        // for a requirement that produced no real diff).
        if let Ok((_code, stdout, _stderr)) = wt_git.run(&["diff", &base]).await {
            return Some(stdout);
        }
    }

    // Fallback only when the base branch is unknown or the diff command failed:
    // show this branch's last commit.
    let diff_arg = format!("{}^1", branch_name);
    match wt_git.run(&["diff", &diff_arg, branch_name]).await {
        Ok((_code, stdout, _stderr)) => Some(stdout),
        Err(_) => Some(String::new()),
    }
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

    // Get the CR's base branch (worktree diffs need it to scope the change).
    let cr: Option<ChangeRequest> = sqlx::query_as("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let target_branch = cr.map(|c| c.target_branch).unwrap_or_default();

    // Live path: the worktree still exists (CR not yet merged / not torn down).
    // Prefer the live diff so in-flight, uncommitted edits show up immediately.
    if let Some(diff) = compute_worktree_diff(
        &session.worktree_path,
        &session.branch_name,
        &target_branch,
        session.base_commit.as_deref(),
    )
    .await
    {
        if !diff.is_empty() {
            return Ok(diff);
        }
        // Live diff was empty: fall through to the persisted snapshot below in
        // case it captured something (covers torn-down vs. genuinely-empty).
    }

    // Fallback: worktree is gone (merged) — return the diff snapshot taken at
    // merge time so already-merged requirements still show their code changes.
    Ok(session.diff_content.unwrap_or_default())
}

/// 合并冲突现场（供审核页三方视图）。
#[derive(serde::Serialize)]
pub struct MergeConflictView {
    pub files: Vec<String>,
    pub diff: String,
}

/// 读取某 CR 在 `merge_conflict` 态记录的冲突现场（文件列表 + 带标记 diff）。
#[tauri::command]
pub async fn get_merge_conflict(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<MergeConflictView, String> {
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT conflict_files, conflict_diff FROM worktree_sessions
         WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let (files_json, diff) = row.unwrap_or((None, None));
    let files = files_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    Ok(MergeConflictView {
        files,
        diff: diff.unwrap_or_default(),
    })
}

/// 一键重试合并：回到 `pending_merge` 并重新入队 Merge 任务。走 Phase 1 自动 merge-dev，
/// 若此时 dev 已含解（或冲突已消）即可干净落地。
#[tauri::command]
pub async fn retry_merge(cr_id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query(
        "UPDATE change_requests SET status='pending_merge', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let cr: ChangeRequest = sqlx::query_as("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE issues SET status='pending_merge', updated_at=datetime('now') WHERE id=?")
        .bind(&cr.issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let _ = enqueue(
        &state.db,
        &state.job_tx,
        "merge",
        &format!("merge:{}:retry:{}", cr_id, Uuid::new_v4()),
        JobPayload::Merge {
            change_request_id: cr_id.clone(),
        },
    )
    .await;
    Ok(())
}

/// 方案 B 手动入口：把当前 CR 的合并冲突交给 AI 自动解决（解完回审核 2 复审）。
/// 长任务，spawn 到后台执行，命令立即返回。
#[tauri::command]
pub async fn ai_resolve_merge_conflict(
    cr_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    let tx = state.job_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::tasks::merge::ai_resolve_conflict(&db, &tx, &app, &cr_id).await {
            tracing::info!("ai_resolve_merge_conflict failed for {}: {}", cr_id, e);
        }
    });
    Ok(())
}

/// Review 1: approve moves to pending_execution and creates a CR + enqueues Execution job
#[tauri::command]
pub async fn review_1(
    issue_id: String,
    decision: Review1Decision,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    if decision.decision == "approved" {
        let admin_id = decision.admin_id.unwrap_or_else(|| "admin".to_string());
        approve_issue_review_1(
            &state.db,
            &state.job_tx,
            &issue_id,
            decision.suggestions.as_deref(),
            &admin_id,
        )
        .await
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

        // Innate: a human rejected this analysis at review_1 — demote the recalled
        // experience that fed it (negative calibration signal).
        crate::knowledge::consume_recall_trace(&state.db, "issue", &issue_id, "fail", Some("down")).await;

        // Rejected
        sqlx::query("UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?")
            .bind(&issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        Err("Issue rejected".to_string())
    }
}

/// Shared review_1 approval path: create the CR, record the gate decision, feed the
/// Innate calibration loop, flip the issue to `pending_execution`, and enqueue the
/// Execution job. Reused by both single `review_1` and batch `review_1_batch` so the
/// two stay behaviourally identical. Pure deps (db + job sender) — no Tauri types.
async fn approve_issue_review_1(
    db: &crate::db::Db,
    job_tx: &crate::tasks::runner::JobSender,
    issue_id: &str,
    suggestions: Option<&str>,
    admin_id: &str,
) -> Result<ChangeRequest, String> {
    // Load issue to get project
    let issue = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
        .bind(issue_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    // Only requirements still awaiting review 1 may be approved (guards stale/batch ids).
    if issue.status != "pending_review_1" {
        return Err(format!("需求当前状态为 {}，不可审核通过", issue.status));
    }

    // Load project for default branch
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&issue.project_id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string())?;

    // Create CR
    let cr_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO change_requests (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch)
         VALUES (?, ?, ?, 'pending_execution', ?, ?, ?)"
    )
    .bind(&cr_id)
    .bind(&issue.project_id)
    .bind(issue_id)
    .bind(admin_id)
    .bind(suggestions.unwrap_or(""))
    .bind(&project.branch_dev)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    record_admin_decision(
        db,
        &issue.project_id,
        issue_id,
        Some(&cr_id),
        "review_1",
        "approved",
        admin_id,
        suggestions,
    )
    .await?;

    // Innate: the analysis a human approved at review_1 was a good call —
    // reinforce the experience that fed it (positive calibration signal).
    crate::knowledge::consume_recall_trace(db, "issue", issue_id, "ok", Some("up")).await;

    // Update issue
    sqlx::query("UPDATE issues SET status='pending_execution', updated_at=datetime('now') WHERE id=?")
        .bind(issue_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;

    // Enqueue execution job
    let idem_key = format!("execution:{}", cr_id);
    let _ = enqueue(
        db,
        job_tx,
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
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

/// Result of a batch review-1 approval: how many requirements were approved,
/// skipped (no longer awaiting review 1), or errored.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Review1BatchResult {
    pub approved: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Review 1 (batch): approve many pending requirements at once to clear the
/// review-1 queue quickly. Each id runs through the same `approve_issue_review_1`
/// path as the single command; ids that are no longer `pending_review_1` are
/// skipped (not errored) so a stale selection doesn't fail the whole batch.
#[tauri::command]
pub async fn review_1_batch(
    issue_ids: Vec<String>,
    suggestions: Option<String>,
    admin_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Review1BatchResult, String> {
    let admin_id = admin_id.unwrap_or_else(|| "admin".to_string());
    let suggestions = suggestions.filter(|s| !s.trim().is_empty());
    let mut result = Review1BatchResult { approved: 0, skipped: 0, errors: 0 };
    for issue_id in issue_ids {
        match approve_issue_review_1(
            &state.db,
            &state.job_tx,
            &issue_id,
            suggestions.as_deref(),
            &admin_id,
        )
        .await
        {
            Ok(_) => result.approved += 1,
            // A requirement that left pending_review_1 (already approved/rejected
            // elsewhere) is skipped rather than counted as a hard error.
            Err(e) if e.contains("不可审核通过") => result.skipped += 1,
            Err(e) => {
                tracing::warn!("review_1_batch: approve {} failed: {}", issue_id, e);
                result.errors += 1;
            }
        }
    }
    Ok(result)
}

/// Feed a review-2 human outcome into the change class trust state machine
/// (a clean approval extends the streak toward auto-pass; reject/revision resets it).
/// Shared by single `review_2` and the batch path so calibration stays identical.
async fn record_review_2_outcome(db: &crate::db::Db, cr_id: &str, approved: bool) {
    let change_class: String = sqlx::query_as::<_, (String,)>(
        "SELECT change_class FROM cr_grades WHERE change_request_id=?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(c,)| c)
    .unwrap_or_else(|| "general".to_string());
    crate::core::gate::record_review_outcome(db, &change_class, approved).await;
}

/// Shared review_2 approval path: feed the change-class calibration, record the gate
/// decision, flip the CR to `pending_merge`, enqueue the Merge job, and release the
/// pending-review slot. Reused by single `review_2` and batch `review_2_batch` so the
/// two stay behaviourally identical. Pure deps (db + job sender + concurrency) — no
/// Tauri types. CRs no longer awaiting review 2 are rejected with an error so a stale
/// batch selection can be skipped (not silently merged).
async fn approve_cr_review_2(
    db: &crate::db::Db,
    job_tx: &crate::tasks::runner::JobSender,
    concurrency: &crate::core::concurrency::ConcurrencyManager,
    cr_id: &str,
    suggestions: Option<&str>,
    admin_id: &str,
) -> Result<ChangeRequest, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    // Only CRs still awaiting review 2 may be approved (guards stale/batch ids).
    if cr.status != "pending_review_2" {
        return Err(format!("变更请求当前状态为 {}，不可审核通过", cr.status));
    }

    record_review_2_outcome(db, cr_id, true).await;

    record_admin_decision(
        db,
        &cr.project_id,
        &cr.issue_id,
        Some(cr_id),
        "review_2",
        "approved",
        admin_id,
        suggestions,
    )
    .await?;

    sqlx::query(
        "UPDATE change_requests SET status='pending_merge', admin_suggestions_2=?, admin_id=?, approved_at=datetime('now'), updated_at=datetime('now') WHERE id=?"
    )
    .bind(suggestions.unwrap_or(""))
    .bind(admin_id)
    .bind(cr_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // Enqueue merge job
    let idem_key = format!("merge:{}", cr_id);
    let _ = enqueue(
        db,
        job_tx,
        "merge",
        &idem_key,
        JobPayload::Merge {
            change_request_id: cr_id.to_string(),
        },
    )
    .await;

    // Release pending review slot
    concurrency.release_pending_review();

    sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
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
        approve_cr_review_2(
            &state.db,
            &state.job_tx,
            &state.concurrency,
            &cr_id,
            decision.suggestions.as_deref(),
            &admin_id,
        )
        .await
    } else if decision.decision == "rejected" {
        // §7: reject resets the change-class trust streak.
        record_review_2_outcome(&state.db, &cr_id, false).await;
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

        // Innate: close the recall feedback loop with a negative signal — the
        // recalled knowledge fed code a human rejected at review 2.
        crate::knowledge::consume_recall_trace(&state.db, "change_request", &cr_id, "fail", Some("down")).await;

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
        // §7: revision resets the change-class trust streak.
        record_review_2_outcome(&state.db, &cr_id, false).await;
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

        // Leaving pending_review_2 back to execution frees the review slot.
        state.concurrency.release_pending_review();

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

/// Result of a batch review-2 approval: how many change requests were approved
/// (queued to merge), skipped (no longer awaiting review 2), or errored.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Review2BatchResult {
    pub approved: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Review 2 (batch): approve many change requests awaiting review 2 at once to clear
/// the code-review queue quickly. Each id runs through the same `approve_cr_review_2`
/// path as the single command; ids that are no longer `pending_review_2` are skipped
/// (not errored) so a stale selection doesn't fail the whole batch.
#[tauri::command]
pub async fn review_2_batch(
    cr_ids: Vec<String>,
    suggestions: Option<String>,
    admin_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Review2BatchResult, String> {
    let admin_id = admin_id.unwrap_or_else(|| "admin".to_string());
    let suggestions = suggestions.filter(|s| !s.trim().is_empty());
    let mut result = Review2BatchResult { approved: 0, skipped: 0, errors: 0 };
    for cr_id in cr_ids {
        match approve_cr_review_2(
            &state.db,
            &state.job_tx,
            &state.concurrency,
            &cr_id,
            suggestions.as_deref(),
            &admin_id,
        )
        .await
        {
            Ok(_) => result.approved += 1,
            // A CR that left pending_review_2 (already merged/rejected elsewhere) is
            // skipped rather than counted as a hard error.
            Err(e) if e.contains("不可审核通过") => result.skipped += 1,
            Err(e) => {
                tracing::warn!("review_2_batch: approve {} failed: {}", cr_id, e);
                result.errors += 1;
            }
        }
    }
    Ok(result)
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

    if cr.status != "execution_failed"
        && cr.status != "merge_failed"
        && cr.status != "merge_conflict"
        && cr.status != "no_change_needed"
    {
        return Err(format!(
            "只有执行失败 / 合并失败 / 合并冲突 / 无需改动的需求可以重新执行（当前状态：{}）",
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
