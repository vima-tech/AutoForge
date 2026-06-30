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

/// 分页查询变更请求（功能审计左栏代码闸用）：按项目 + 状态过滤，按 created_at 倒序。
/// 已合并 CR 会随项目生命周期无限累积，全量加载会拖垮列表，故走分页：
/// - 活动集（非合并）天然有界，调用方一次取够（大 limit）；
/// - 已合并历史按页滚动加载，total 同时供左栏「已合并」徽标计数。
///   status 为空或 "all" 表示不过滤状态；exclude_merged 为 true 时排除已合并
///   （仅在未显式按 merged 筛选时生效，显式选 merged 说明主动要看）。
#[derive(serde::Serialize)]
pub struct ChangeRequestPage {
    pub items: Vec<ChangeRequest>,
    pub total: i64,
}

#[tauri::command]
pub async fn list_change_requests_page(
    project_id: Option<String>,
    status: Option<String>,
    exclude_merged: Option<bool>,
    limit: i64,
    offset: i64,
    state: State<'_, AppState>,
) -> Result<ChangeRequestPage, String> {
    use sqlx::{QueryBuilder, Sqlite};
    let limit = limit.clamp(1, 1000);
    let offset = offset.max(0);
    let status = status.filter(|s| !s.is_empty() && s != "all");
    let exclude_merged =
        exclude_merged.unwrap_or(false) && status.as_deref() != Some("merged");

    // WHERE 子句在 COUNT 与取数两处各自内联拼装（push_bind 绑定值须与各自 qb 同寿命，
    // 故不抽成闭包），两处条件保持一致。
    let total: i64 = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM change_requests WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        qb.build_query_scalar()
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let items = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT * FROM change_requests WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);
        qb.build_query_as::<ChangeRequest>()
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(ChangeRequestPage { items, total })
}

/// 按需求 id 取其最新的变更请求——用于功能审计左栏分批加载后，从总账下钻到
/// 一个尚未载入的（如较早的已合并）CR 时按需补拉单条，恢复全量加载时的下钻能力。
#[tauri::command]
pub async fn get_change_request_by_issue(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ChangeRequest>, String> {
    sqlx::query_as::<_, ChangeRequest>(
        "SELECT * FROM change_requests WHERE issue_id=? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&issue_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())
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

/// 代码审核页「批准合并」前预填的默认合并提交信息。
///
/// 与后端合并任务回退时使用的模板（`tasks::merge::default_merge_message`）共享同一实现，
/// 保证审核页预填内容与人审留空时实际落地的提交信息字字一致（单一真源）。
#[tauri::command]
pub async fn get_default_merge_message(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("cr {} not found", cr_id))?;
    Ok(crate::tasks::merge::default_merge_message(&state.db, &cr).await)
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
    load_cr_diff(&state.db, &cr_id).await
}

/// 计算某 CR 的完整代码 diff（纯 Rust，零 Tauri 类型）。优先取 worktree 实时 diff，
/// worktree 已销毁（合并后）则回退合并时落库的快照。供 `get_code_diff` 命令与
/// `change_summary` 等内部消费者复用，避免重复 git 调用 / 大 diff 在 IPC 上往返两次。
pub async fn load_cr_diff(db: &crate::db::Db, cr_id: &str) -> Result<String, String> {
    // Get worktree session
    let session: Option<WorktreeSession> = sqlx::query_as(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;

    let session = match session {
        Some(s) => s,
        None => return Ok(String::new()),
    };

    // Get the CR's base branch (worktree diffs need it to scope the change).
    let cr: Option<ChangeRequest> = sqlx::query_as("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
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
    // 状态守卫（对齐 revert_change_request 的原子门）：仅从失败态重试，避免把已合并/合并中/
    // 其他状态的 CR 错误重置为 pending_merge 并重新入队（覆盖有效状态、重复合并）。
    let guarded = sqlx::query(
        "UPDATE change_requests SET status='pending_merge', updated_at=datetime('now')
         WHERE id=? AND status IN ('merge_failed','merge_conflict')",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if guarded.rows_affected() == 0 {
        return Err("仅合并失败/冲突的需求可重试合并（可能已合并或状态已变化）".to_string());
    }
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
    // 入队合并流水线（开关 ON 时重试也走并行 premerge：在最新 dev 上重测后再落地）。
    crate::tasks::merge::enqueue_merge_pipeline(&state.db, &state.job_tx, &cr_id, "retry").await;
    Ok(())
}

/// 撤销一个已合并需求的改动：在 dev 上 `git revert` 其 squash 提交。
///
/// 原子状态门（merged→reverting）防双击重复入队（enqueue 的幂等键只防重复行、不防重复
/// 执行）；无 `merge_commit` 的旧 CR（合并早于撤销功能 / 空改动）直接拒绝。实际撤销在
/// `tasks/revert.rs` 后台 job 内执行，复用合并锁 + per-CR 锁串行。
#[tauri::command]
pub async fn revert_change_request(cr_id: String, state: State<'_, AppState>) -> Result<(), String> {
    // 必须有可撤销的 squash 提交 SHA。
    let has_commit = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT merge_commit FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .and_then(|(c,)| c)
    .map(|c| !c.trim().is_empty())
    .unwrap_or(false);
    if !has_commit {
        return Err("该需求没有可撤销的合并提交（合并早于撤销功能或为空改动）".to_string());
    }
    // 原子门：仅 merged 可转 reverting；二次点击/竞态看到非 merged 即拒绝。
    let res = sqlx::query(
        "UPDATE change_requests SET status='reverting', updated_at=datetime('now') WHERE id=? AND status='merged'",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err("仅已合并的需求可撤销（可能已在撤销中或状态已变化）".to_string());
    }
    // 合并 CR：撤销整条 squash 提交即撤销全部成员需求 → 全组置 reverting。
    set_cr_issues_status(&state.db, &cr_id, "reverting")
        .await
        .map_err(|e| e.to_string())?;
    // 唯一 key：去重靠上面的状态门，而非 enqueue 幂等键。
    let _ = enqueue(
        &state.db,
        &state.job_tx,
        "revert",
        &format!("revert:{}:{}", cr_id, Uuid::new_v4()),
        JobPayload::Revert {
            change_request_id: cr_id.clone(),
        },
    )
    .await;
    Ok(())
}

/// 方案 B 手动入口：把当前 CR 的合并冲突交给 AI 自动解决（解完回代码审核 复审）。
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
            AdminDecisionRecord {
                project_id: &issue.project_id,
                issue_id: &issue_id,
                change_request_id: None,
                stage: "review_1",
                decision: "rejected",
                admin_id: &admin_id,
                suggestions: decision.suggestions.as_deref(),
            },
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

    // Requirements awaiting review 1 — and those whose auto-analysis FAILED — may be
    // approved straight into coding. Approving an `analysis_failed` issue deliberately
    // skips the (missing/empty) analysis spec; execution still runs off the raw issue
    // description. Other states (executing/merged/rejected…) are guarded against
    // stale/batch ids. The "不可审核通过" error string is matched by review_1_batch to
    // skip (not error) such ids.
    if issue.status != "pending_issue_review" && issue.status != "analysis_failed" {
        return Err(format!("需求当前状态为 {}，不可审核通过", issue.status));
    }

    // Load project for default branch
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&issue.project_id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string())?;

    // Innate: the analysis a human approved at review_1 was a good call —
    // reinforce the experience that fed it (positive calibration signal).
    crate::knowledge::consume_recall_trace(db, "issue", issue_id, "ok", Some("up")).await;

    create_cr_for_issue(db, job_tx, &issue, &project, suggestions, admin_id, None).await
}

/// Create a CR for an already-loaded issue and enqueue its Execution job: insert the
/// CR row (carrying optional express `work_context`), write the `primary` association
/// row, record the `review_1` approval decision, flip the issue to `pending_execution`,
/// and enqueue execution under the stable `execution:<cr_id>` idempotency key.
///
/// Shared by the normal review_1 path (`approve_issue_review_1`, `work_context=None`)
/// and the 会议室「立即编码」express path (`start_conversation_coding`, which passes the
/// conversation snapshot as `work_context` and skips the review_1 *queue* — the operator's
/// click in the room IS the requirement-side decision; code review (review_2) still gates
/// the merge). Pure deps (db + job sender) — no Tauri types.
pub(crate) async fn create_cr_for_issue(
    db: &crate::db::Db,
    job_tx: &crate::tasks::runner::JobSender,
    issue: &crate::models::issue::Issue,
    project: &crate::models::project::Project,
    suggestions: Option<&str>,
    admin_id: &str,
    work_context: Option<&str>,
) -> Result<ChangeRequest, String> {
    let issue_id = issue.id.as_str();
    let cr_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO change_requests (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch, work_context)
         VALUES (?, ?, ?, 'pending_execution', ?, ?, ?, ?)"
    )
    .bind(&cr_id)
    .bind(&issue.project_id)
    .bind(issue_id)
    .bind(admin_id)
    .bind(suggestions.unwrap_or(""))
    .bind(&project.branch_dev)
    .bind(work_context)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 关联表 primary 行：单需求 CR 也写，保证「CR 的全部需求」恒等于 change_request_issues 查询。
    add_cr_issue(db, &cr_id, issue_id, "primary", 0, "auto").await?;

    record_admin_decision(
        db,
        AdminDecisionRecord {
            project_id: &issue.project_id,
            issue_id,
            change_request_id: Some(&cr_id),
            stage: "review_1",
            decision: "approved",
            admin_id,
            suggestions,
        },
    )
    .await?;

    // Update issue
    sqlx::query("UPDATE issues SET status='pending_execution', updated_at=datetime('now') WHERE id=?")
        .bind(issue_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;

    // Enqueue execution job。不再吞掉 enqueue 错误（旧码 `let _ =` 会让 job_executions 写失败
    // 时悄无声息地留下「issue=pending_execution、CR 已建、却无执行任务」的孤儿 CR）。
    // 现在向上抛出失败让调用方可见；即便此处失败，启动恢复（recover_orphaned_executions）
    // 也会按幂等键 `execution:<cr>` 重新入队，孤儿 CR 会在下次启动自愈。
    let idem_key = format!("execution:{}", cr_id);
    enqueue(
        db,
        job_tx,
        "execution",
        &idem_key,
        JobPayload::Execution {
            change_request_id: cr_id.clone(),
            project_id: issue.project_id.clone(),
        },
    )
    .await
    .map_err(|e| format!("CR 已创建但执行任务入队失败（将于下次启动自动重排）：{e}"))?;

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
/// path as the single command; ids that are no longer `pending_issue_review` are
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
            // A requirement that left pending_issue_review (already approved/rejected
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

/// 一个 CR 覆盖的需求引用（合并 CR 含多条；普通 CR 一条）。供审核页展示「覆盖 N 个需求」。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrIssueRef {
    pub issue_id: String,
    pub title: String,
    pub role: String,
    pub status: String,
}

/// 列出某 CR 覆盖的全部需求（primary 在前），供前端展示「本变更覆盖 N 个需求」。
#[tauri::command]
pub async fn get_change_request_issues(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<CrIssueRef>, String> {
    let mut out = Vec::new();
    for id in cr_issue_ids(&state.db, &cr_id).await {
        if let Some((title, status)) =
            sqlx::query_as::<_, (String, String)>("SELECT title, status FROM issues WHERE id=?")
                .bind(&id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| e.to_string())?
        {
            let role: Option<(String,)> = sqlx::query_as(
                "SELECT role FROM change_request_issues WHERE change_request_id=? AND issue_id=?",
            )
            .bind(&cr_id)
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            out.push(CrIssueRef {
                issue_id: id,
                title,
                role: role.map(|(r,)| r).unwrap_or_else(|| "primary".to_string()),
                status,
            });
        }
    }
    Ok(out)
}

/// 合并需求审核：把多条**待需求审核**的需求合并成**一个** CR + 一次执行（同文件多需求合并，
/// 亦即人工批量绑定的工单组）。primary 需求驱动 CR 标题/分支/Innate 召回，其余为 member。
/// 全部成员翻 `pending_execution` 并各记一条 review_1 审计，最后入队**一个** Execution job
/// （执行层会拼多需求工单）。
/// 校验：≥2 条、同项目、均为 `pending_issue_review`（或分析失败，可直接进编码）、
/// 不超过 `MAX_GROUP` 上限；人工绑定（bind_source="manual"）相关性不足时需 `force_unrelated`
/// 二次确认（前端先调 `preview_batch_bind` 渲染信号，此处为服务端安全网）。
#[tauri::command]
pub async fn review_1_merge(
    issue_ids: Vec<String>,
    suggestions: Option<String>,
    primary_id: Option<String>,
    admin_id: Option<String>,
    bind_source: Option<String>,
    force_unrelated: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    let admin_id = admin_id.unwrap_or_else(|| "admin".to_string());
    let suggestions = suggestions.filter(|s| !s.trim().is_empty());
    let db = &state.db;

    // 去重，保持选择顺序。
    let mut seen = std::collections::HashSet::new();
    let ordered: Vec<String> = issue_ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if ordered.len() < 2 {
        return Err("合并至少需要 2 条需求".to_string());
    }

    // 加载全部需求，校验状态 + 同项目。
    let mut issues = Vec::with_capacity(ordered.len());
    for id in &ordered {
        let iss = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("需求不存在：{}", id))?;
        if iss.status != "pending_issue_review" && iss.status != "analysis_failed" {
            return Err(format!("需求「{}」状态为 {}，不可合并", iss.title, iss.status));
        }
        issues.push(iss);
    }
    let project_id = issues[0].project_id.clone();
    if !issues.iter().all(|i| i.project_id == project_id) {
        return Err("只能合并同一项目下的需求".to_string());
    }

    // 工单组成员上限护栏（设计评审 D3：MAX_GROUP=5），控制爆炸半径与风险连坐。
    if ordered.len() > crate::commands::requirement_merge::MAX_GROUP {
        return Err(format!(
            "工单组最多 {} 条需求，当前选了 {} 条",
            crate::commands::requirement_merge::MAX_GROUP,
            ordered.len()
        ));
    }

    // 相关度服务端安全网：人工绑定（manual）且相关性「真无关 / 数据不足」时，必须显式
    // force_unrelated 才放行。前端已先调 preview_batch_bind 渲染信号 + 二次确认（D5），
    // 此处仅防绕过预览。'auto'（规则探测推荐采纳）天然相关，不设此门。
    let bind_source = bind_source
        .filter(|s| s == "manual")
        .unwrap_or_else(|| "auto".to_string());
    if bind_source == "manual" && force_unrelated != Some(true) {
        let files = crate::commands::requirement_merge::load_issue_files(db, &ordered).await?;
        let rel = crate::commands::requirement_merge::group_relatedness(&files);
        if matches!(rel.signal, "unrelated" | "insufficient") {
            return Err("所选需求相关性不足，请在预览确认后再绑定".to_string());
        }
    }

    // 选 primary：用传入的（且在选区内）否则取第一条。
    let primary = primary_id
        .filter(|p| ordered.iter().any(|id| id == p))
        .unwrap_or_else(|| ordered[0].clone());

    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_one(db)
            .await
            .map_err(|e| e.to_string())?;

    // 创建一个 CR（issue_id = primary）。
    let cr_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO change_requests (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch)
         VALUES (?, ?, ?, 'pending_execution', ?, ?, ?)"
    )
    .bind(&cr_id)
    .bind(&project_id)
    .bind(&primary)
    .bind(&admin_id)
    .bind(suggestions.as_deref().unwrap_or(""))
    .bind(&project.branch_dev)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 写关联表：primary + members（members 按选择顺序排 sort_order）。
    let mut order = 1i64;
    for id in &ordered {
        if *id == primary {
            add_cr_issue(db, &cr_id, id, "primary", 0, &bind_source).await?;
        } else {
            add_cr_issue(db, &cr_id, id, "member", order, &bind_source).await?;
            order += 1;
        }
    }

    // 全部成员：记 review_1 审计 + Innate 正校准 + 翻 pending_execution（走全组联动助手）。
    for id in &ordered {
        record_admin_decision(
            db,
            AdminDecisionRecord {
                project_id: &project_id,
                issue_id: id,
                change_request_id: Some(&cr_id),
                stage: "review_1",
                decision: "approved",
                admin_id: &admin_id,
                suggestions: suggestions.as_deref(),
            },
        )
        .await?;
        crate::knowledge::consume_recall_trace(db, "issue", id, "ok", Some("up")).await;
    }
    set_cr_issues_status(db, &cr_id, "pending_execution")
        .await
        .map_err(|e| e.to_string())?;

    // 入队一个 Execution job（执行层据关联表拼多需求工单）。
    let idem_key = format!("execution:{}", cr_id);
    let _ = enqueue(
        db,
        &state.job_tx,
        "execution",
        &idem_key,
        JobPayload::Execution {
            change_request_id: cr_id.clone(),
            project_id: project_id.clone(),
        },
    )
    .await;

    sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

/// 从一个**尚未进入执行**（pending_execution）的合并 CR 中拆出某成员需求，退回
/// `pending_issue_review` 重新独立审核。应对「合并后想撤回其中一条」。
/// 仅允许在执行前拆（执行后 worktree/diff 已成型，拆分语义复杂，不在 MVP 内）；
/// 不允许拆 primary（primary 驱动 CR 身份）；拆到只剩 1 条时不自动解散 CR（保持简单）。
#[tauri::command]
pub async fn split_change_request(
    cr_id: String,
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = &state.db;
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("变更请求不存在：{}", cr_id))?;
    if cr.status != "pending_execution" {
        return Err(format!("变更请求当前状态为 {}，仅可在进入执行前拆分", cr.status));
    }
    if cr.issue_id == issue_id {
        return Err("不可拆分主需求（primary）".to_string());
    }
    let members = cr_issue_ids(db, &cr_id).await;
    if !members.contains(&issue_id) {
        return Err("该需求不属于此变更请求".to_string());
    }

    // 移除关联 + 需求退回独立审核。
    sqlx::query("DELETE FROM change_request_issues WHERE change_request_id=? AND issue_id=?")
        .bind(&cr_id)
        .bind(&issue_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE issues SET status='pending_issue_review', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&issue_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 从一个**已进入代码审核**（pending_code_review）的合并 CR 中摘出某成员需求：该需求退回
/// 独立审核（pending_issue_review，保留分析结果），剩余成员作废当前 diff、**重新入队执行**
/// 用收窄后的需求集重跑。解决 squash 下「一个错全组陪葬」——把全有或全无的代价降到最小。
///
/// 设计评审修正（务必照此实现，勿自创机制）：
///  - **D1**：仅允许 `pending_code_review` 态摘出（唯一持有 pending_review 槽的态）→ 必
///    `release_pending_review()` 一次，单一来源无双重释放风险；merge_conflict/execution_failed
///    走既有 closure 路径，不在此处。
///  - **D2**：重执行用 `execution:{cr}:retry:{uuid}` 唯一键（enqueue 既去重行也去重执行，
///    普通键会撞已 completed 行致**静默不重跑**）——与 review_2 revision 路径完全一致。
///  - 不可摘 primary（驱动 CR 身份）；摘到只剩 primary 一条 → 退化为单需求 CR 正常重跑。
///  - 剩余组自动收窄：execution.rs 每次**实时**读 change_request_issues，DELETE 后被摘需求
///    自然不在 member 集，`merged_requirements` 随之收窄（这正是 detach 能 work 的支点）。
#[tauri::command]
pub async fn detach_and_requeue(
    cr_id: String,
    issue_id: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    let db = &state.db;
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("变更请求不存在：{}", cr_id))?;
    // D1：仅代码审核阶段（唯一持有 pending_review 槽的态）可摘出。
    if cr.status != "pending_code_review" {
        return Err(format!(
            "变更请求当前状态为 {}，仅可在代码审核阶段摘出需求",
            cr.status
        ));
    }
    if cr.issue_id == issue_id {
        return Err("不可摘出主需求（primary）".to_string());
    }
    let members = cr_issue_ids(db, &cr_id).await;
    if !members.contains(&issue_id) {
        return Err("该需求不属于此变更请求".to_string());
    }

    // 1) 摘成员关联 + 需求退回独立审核（issue_analyses 保留，可直接重新合并/独立编码）。
    sqlx::query("DELETE FROM change_request_issues WHERE change_request_id=? AND issue_id=?")
        .bind(&cr_id)
        .bind(&issue_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE issues SET status='pending_issue_review', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&issue_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    // 审计：被摘需求记一条 review_2 'detached'（reason 入 suggestions）。
    record_admin_decision(
        db,
        AdminDecisionRecord {
            project_id: &cr.project_id,
            issue_id: &issue_id,
            change_request_id: Some(&cr_id),
            stage: "review_2",
            decision: "detached",
            admin_id: cr.admin_id.as_deref().unwrap_or("admin"),
            suggestions: reason.as_deref(),
        },
    )
    .await?;

    // 2) 收尾完全照抄 review_2 revision 路径（释放审核槽 → 退回执行 → 唯一键重入队 → emit）。
    state.concurrency.release_pending_review(); // D1：单一来源，释放一次
    sqlx::query(
        "UPDATE change_requests SET status='pending_execution', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&cr_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    // 剩余成员需求一并回 pending_execution（被摘需求已不在关联表，不受影响）。
    set_cr_issues_status(db, &cr_id, "pending_execution")
        .await
        .map_err(|e| e.to_string())?;

    let idem_key = format!("execution:{}:retry:{}", cr_id, Uuid::new_v4()); // D2
    let _ = enqueue(
        db,
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
            message: Some("已摘出需求，剩余组重新执行".to_string()),
        },
    );

    sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
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
    commit_message: Option<&str>,
    admin_id: &str,
) -> Result<ChangeRequest, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    // Only CRs still awaiting review 2 may be approved (guards stale/batch ids).
    if cr.status != "pending_code_review" {
        return Err(format!("变更请求当前状态为 {}，不可审核通过", cr.status));
    }

    // 规整人工提交信息：去空白、空串落 NULL（合并任务回退默认模板）、限长 2KB。
    // 仅在「自定义提交信息」开关开启时采纳；关闭时强制 NULL，合并走默认模板。
    let merge_msg = if crate::core::gate::custom_merge_message_enabled(db).await {
        commit_message
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(2000).collect::<String>())
    } else {
        None
    };

    // 原子认领：仅 pending_code_review→pending_merge，且只有一个并发调用能 rows_affected>0。
    // 上面的预检查只给出友好报错；真正的并发门是这条带 status 条件的 UPDATE——否则两次几乎
    // 同时的审核通过会双双越过预检查，各自重复记审计、重复入队合并、并**两次** release_pending_review
    // （污染并发计数器，使其虚高、永久挤占审核槽位）。
    let claimed = sqlx::query(
        "UPDATE change_requests SET status='pending_merge', admin_suggestions_2=?, merge_commit_message=?, admin_id=?, approved_at=datetime('now'), updated_at=datetime('now') WHERE id=? AND status='pending_code_review'"
    )
    .bind(suggestions.unwrap_or(""))
    .bind(&merge_msg)
    .bind(admin_id)
    .bind(cr_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    if claimed.rows_affected() == 0 {
        return Err("变更请求状态已变化，不可重复审核通过".to_string());
    }

    // 以下副作用仅「认领成功」的那一个调用执行一次。
    record_review_2_outcome(db, cr_id, true).await;
    // 合并 CR：全部成员需求各记一条「review_2 approved」审计。
    record_admin_decision_all(db, &cr.project_id, cr_id, "review_2", "approved", admin_id, suggestions)
        .await?;

    // 入队合并流水线（开关 ON → 并行 premerge；OFF → legacy merge）。统一唯一键，杜绝撞
    // 历史 completed 行被去重不派发（CR 卡死 pending_merge 的根因）。
    crate::tasks::merge::enqueue_merge_pipeline(db, job_tx, cr_id, "approve").await;

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
            decision.commit_message.as_deref(),
            &admin_id,
        )
        .await
    } else if decision.decision == "rejected" {
        // §7: reject resets the change-class trust streak.
        record_review_2_outcome(&state.db, &cr_id, false).await;
        record_admin_decision_all(
            &state.db,
            &cr.project_id,
            &cr_id,
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

        // 合并 CR：全部成员需求一并置 rejected（单需求 CR 等价于只动那一条）。
        set_cr_issues_status(&state.db, &cr_id, "rejected")
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
            AdminDecisionRecord {
                project_id: &cr.project_id,
                issue_id: &cr.issue_id,
                change_request_id: Some(&cr_id),
                stage: "review_2",
                decision: "revision",
                admin_id: &admin_id,
                suggestions: decision.suggestions.as_deref(),
            },
        )
        .await?;

        // Leaving pending_code_review back to execution frees the review slot.
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
/// path as the single command; ids that are no longer `pending_code_review` are skipped
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
            None, // 批量审核不逐条填提交信息，合并任务回退默认模板
            &admin_id,
        )
        .await
        {
            Ok(_) => result.approved += 1,
            // A CR that left pending_code_review (already merged/rejected elsewhere) is
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

/// Worktree/preview cleanup by CR id. The startup reconciler
/// (`tasks::runner::requeue_orphaned_executions`) only has the id, so it loads the
/// CR then delegates to [`cleanup_cr_worktrees`].
pub(crate) async fn cleanup_cr_worktrees_by_id(db: &crate::db::Db, cr_id: &str) {
    if let Ok(cr) = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_one(db)
        .await
    {
        cleanup_cr_worktrees(db, &cr).await;
    }
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
    // 合并 CR：全部成员需求一并回到 pending_execution。
    set_cr_issues_status(&state.db, &cr_id, "pending_execution")
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

/// Restore a reverted requirement back into the pipeline queue: re-implement it
/// from a clean worktree against the current dev (which already carries the revert
/// commit), so it flows through code review (review_2) and re-merges. Mirrors
/// `retry_change_request`'s reset+enqueue, but gated on the `reverted` terminal
/// state — the closure path for "I undid this, now I want it back".
#[tauri::command]
pub async fn restore_change_request(
    cr_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // Atomic gate: only a reverted CR can re-enter; a double click / race that sees
    // any other state is rejected (the row already moved on).
    let res = sqlx::query(
        "UPDATE change_requests SET status='pending_execution', updated_at=datetime('now') WHERE id=? AND status='reverted'",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err(format!(
            "只有已撤销的需求可以恢复（当前状态：{}）",
            cr.status
        ));
    }

    // Clear leftover worktrees/previews from the prior attempt before re-running.
    cleanup_cr_worktrees(&state.db, &cr).await;

    // Flag the issue(s) as restored-from-revert so the queue can mark them with a small dot.
    // 合并 CR：全部成员需求一并恢复。
    sqlx::query(
        "UPDATE issues SET status='pending_execution', restored_from_revert=1, updated_at=datetime('now')
         WHERE id IN (SELECT issue_id FROM change_request_issues WHERE change_request_id=?)
            OR id = ?",
    )
    .bind(&cr_id)
    .bind(&cr.issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Unique idempotency key so the restore is never swallowed by INSERT OR IGNORE.
    let idem_key = format!("execution:{}:restore:{}", cr_id, Uuid::new_v4());
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
            message: Some("已撤销的需求已重新进入执行队列".to_string()),
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
    if cr.status == "pending_code_review" {
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
    // Merge/scan test gates leave test_sessions rows whose change_request_id is a
    // FK to change_requests. With PRAGMA foreign_keys=ON these block the CR delete
    // below — which is exactly why merge-failed CRs (always tested) couldn't be
    // removed. scan_findings.test_session_id is in turn a NOT NULL FK to those
    // sessions, so clear the findings before the sessions before the CR.
    let _ = sqlx::query(
        "DELETE FROM scan_findings WHERE test_session_id IN (SELECT id FROM test_sessions WHERE change_request_id=?)",
    )
    .bind(&cr_id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM test_sessions WHERE change_request_id=?")
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
    // CR-keyed side data without a FK constraint: doesn't block the delete but
    // would linger as orphans, so clean it too (matches this command's intent).
    for tbl in [
        "security_audits",
        "deployments",
        "kb_traces",
        "code_agent_run_logs",
    ] {
        let _ = sqlx::query(&format!("DELETE FROM {tbl} WHERE change_request_id=?"))
            .bind(&cr_id)
            .execute(&state.db)
            .await;
    }
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
    // Sever issue-side FKs that would otherwise block (or be left dangling by)
    // the issue delete: scan findings that spawned it, and duplicate-of links
    // from other issues' analyses pointing at this one. Null rather than delete
    // so unrelated scan/analysis data survives.
    let _ = sqlx::query("UPDATE scan_findings SET issue_entry_id=NULL WHERE issue_entry_id=?")
        .bind(&cr.issue_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query("UPDATE issue_analyses SET duplicate_of=NULL WHERE duplicate_of=?")
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

/// Fields describing a single human审核 decision, grouped to keep
/// `record_admin_decision` under clippy's argument-count threshold.
struct AdminDecisionRecord<'a> {
    project_id: &'a str,
    issue_id: &'a str,
    change_request_id: Option<&'a str>,
    stage: &'a str,
    decision: &'a str,
    admin_id: &'a str,
    suggestions: Option<&'a str>,
}

async fn record_admin_decision(
    db: &crate::db::Db,
    record: AdminDecisionRecord<'_>,
) -> Result<(), String> {
    let AdminDecisionRecord {
        project_id,
        issue_id,
        change_request_id,
        stage,
        decision,
        admin_id,
        suggestions,
    } = record;
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

// ── 合并 CR 的成员需求关联（change_request_issues，迁移 0070）────────────────────

/// 给某 CR 关联一条成员需求（role: 'primary'|'member'）。INSERT OR IGNORE 幂等。
pub(crate) async fn add_cr_issue(
    db: &crate::db::Db,
    cr_id: &str,
    issue_id: &str,
    role: &str,
    sort_order: i64,
    bind_source: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR IGNORE INTO change_request_issues (change_request_id, issue_id, role, sort_order, bind_source)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(cr_id)
    .bind(issue_id)
    .bind(role)
    .bind(sort_order)
    .bind(bind_source)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// CR 的全部成员需求 id（primary 在前，再按 sort_order）。关联表由迁移 0070 回填保证非空；
/// 极端缺失时回落到 change_requests.issue_id，永不返回空集，防止全组联动退化为空操作。
pub(crate) async fn cr_issue_ids(db: &crate::db::Db, cr_id: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT issue_id FROM change_request_issues WHERE change_request_id=?
         ORDER BY CASE role WHEN 'primary' THEN 0 ELSE 1 END, sort_order, issue_id",
    )
    .bind(cr_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    if !rows.is_empty() {
        return rows.into_iter().map(|(id,)| id).collect();
    }
    // 兜底：关联表无行（理论不应发生）时用 CR 主需求。
    sqlx::query_as::<_, (String,)>("SELECT issue_id FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|(id,)| vec![id])
        .unwrap_or_default()
}

/// 把某 CR 的**全部成员需求**置为同一状态（合并 CR 全组联动；单需求 CR 只动那一条）。
/// 关联表是真源，外加 OR id=(CR.issue_id) 兜底，确保即便关联行缺失也至少同步主需求。
pub(crate) async fn set_cr_issues_status(
    db: &crate::db::Db,
    cr_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE issues SET status=?, updated_at=datetime('now')
         WHERE id IN (SELECT issue_id FROM change_request_issues WHERE change_request_id=?)
            OR id = (SELECT issue_id FROM change_requests WHERE id=?)",
    )
    .bind(status)
    .bind(cr_id)
    .bind(cr_id)
    .execute(db)
    .await?;
    Ok(())
}

/// 对某 CR 的**全部成员需求**各记一条审核决策（合并 CR 的完整审计轨迹）。
/// 单需求 CR 退化为一条，与改造前一致。
async fn record_admin_decision_all(
    db: &crate::db::Db,
    project_id: &str,
    cr_id: &str,
    stage: &str,
    decision: &str,
    admin_id: &str,
    suggestions: Option<&str>,
) -> Result<(), String> {
    for iid in cr_issue_ids(db, cr_id).await {
        record_admin_decision(
            db,
            AdminDecisionRecord {
                project_id,
                issue_id: &iid,
                change_request_id: Some(cr_id),
                stage,
                decision,
                admin_id,
                suggestions,
            },
        )
        .await?;
    }
    Ok(())
}
