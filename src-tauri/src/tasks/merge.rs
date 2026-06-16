use crate::core::{event, git::GitProxy};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::state::worktrees_base;
use crate::tasks::runner::{enqueue, JobSender};
use anyhow::{anyhow, Result};
use tracing::info;

/// Land the CR branch on the integration (dev) branch.
///
/// Returns `Ok(())` once the change has reached dev — either committed in place
/// (normal projects) or pushed to `origin/<dev>` (AutoForge managing its own
/// live repo) — or `Err((exit_code, message))` describing why it didn't.
///
/// `dev_is_live` is true when `<dev>` is the branch currently checked out in the
/// project's MAIN working tree. That is exactly the self-hosting case: a plain
/// `checkout dev && merge` would rewrite the very files the running dev server
/// is built from (Vite HMR storm → the whole Tauri shell crashes) and fight the
/// user's uncommitted edits. So for that case we merge inside a throwaway
/// worktree based on the latest `origin/<dev>` and push the result to
/// `origin/<dev>`, never touching the live working tree. All OTHER projects keep
/// the original fast in-place merge.
async fn land_on_dev(
    git: &GitProxy,
    project: &crate::models::project::Project,
    session: &crate::models::worktree::WorktreeSession,
    cr_id: &str,
    dev_is_live: bool,
) -> std::result::Result<(), (i32, String)> {
    let merge_msg = format!("AutoForge merge: {}", cr_id);

    if !dev_is_live {
        // Fast path (all non-self-managed projects): in-place checkout + merge.
        let (cc, _, ce) = git
            .run(&["checkout", &project.branch_dev])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if cc != 0 {
            return Err((cc, format!("checkout {} 失败：{}", project.branch_dev, ce)));
        }
        let (mc, _, me) = git
            .run(&[
                "merge",
                "--no-ff",
                &session.branch_name,
                "-m",
                &merge_msg,
            ])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if mc != 0 {
            // Abort the half-applied merge so dev/worktree stay clean for a retry.
            let _ = git.run(&["merge", "--abort"]).await;
            return Err((mc, me));
        }
        return Ok(());
    }

    // Isolated path (AutoForge self-managed: dev is checked out live).
    // Refresh the remote tracking ref so we base the merge on the newest dev and
    // the push is a fast-forward (best-effort: offline falls back to local dev).
    let _ = git.run(&["fetch", "origin", &project.branch_dev]).await;
    let remote_ref = format!("origin/{}", project.branch_dev);
    let base = if git
        .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false)
    {
        remote_ref
    } else {
        project.branch_dev.clone()
    };

    let tmp_branch = format!("autoforge/merge-{}", cr_id);
    let tmp_path = format!("{}/.merge-{}", worktrees_base(), cr_id);
    // Clear any stale worktree/branch left by a previous failed attempt.
    let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
    let _ = git.run(&["worktree", "prune"]).await;
    let _ = git.run(&["branch", "-D", &tmp_branch]).await;

    let (wc, _, we) = git
        .run(&["worktree", "add", "-b", &tmp_branch, &tmp_path, &base])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if wc != 0 {
        return Err((wc, format!("创建隔离合并 worktree 失败：{}", we)));
    }

    let tmp_git = GitProxy::new(&tmp_path);
    let (mc, _, me) = tmp_git
        .run(&[
            "merge",
            "--no-ff",
            &session.branch_name,
            "-m",
            &merge_msg,
        ])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if mc != 0 {
        let _ = tmp_git.run(&["merge", "--abort"]).await;
        let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
        let _ = git.run(&["branch", "-D", &tmp_branch]).await;
        return Err((mc, me));
    }

    // Push the merge to origin/<dev>. This is what makes it durable, since the
    // throwaway worktree and temp branch are removed immediately after.
    let push_target = format!("HEAD:{}", project.branch_dev);
    let (pc, _, pe) = tmp_git
        .run(&["push", "origin", &push_target])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if pc != 0 {
        // Keep the temp branch as a recoverable backup; only drop the worktree dir.
        let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
        return Err((
            pc,
            format!(
                "合并成功但推送 origin/{} 失败（请检查 SSH/网络）。改动已保留在本地分支 `{}`，可手动恢复。\n{}",
                project.branch_dev, tmp_branch, pe
            ),
        ));
    }

    // Success: tear down the throwaway worktree + temp branch. Local dev stays
    // put; the user adopts the change via the in-app "同步更新" (ff-only pull).
    let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
    let _ = git.run(&["branch", "-D", &tmp_branch]).await;
    Ok(())
}

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

    // Quality gate: run configured tests on the un-merged worktree branch
    // FIRST. A failing gate must block the merge (spec: testing.md).
    let passed = crate::tasks::testing::run_and_gate(db, tx, app, cr_id).await?;
    if !passed {
        info!("pre-merge tests failed for cr {}, blocking merge", cr_id);
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
                message: Some("合并前测试未通过，已阻断合并".to_string()),
            },
        );
        return Ok(());
    }

    let git = GitProxy::new(&project.repo_path);

    // Detect whether dev is the branch currently checked out in the project's
    // main working tree. When it is (AutoForge managing its own running repo),
    // we must NOT merge in place — see `land_on_dev`. `branch --show-current`
    // prints empty for a detached HEAD, so this stays false off-dev.
    let live_branch = git
        .run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default();
    let dev_is_live = !live_branch.is_empty() && live_branch == project.branch_dev;

    if let Err((merge_code, merge_err)) =
        land_on_dev(&git, &project, &session, cr_id, dev_is_live).await
    {
        info!("merge failed ({}): {}", merge_code, merge_err);
        let fail_reason = format!(
            "## 合并失败\n\n无法将分支 `{}` 合并到 `{}`（退出码 {}）。常见原因为代码冲突，可在修复后重新执行。\n\n```\n{}\n```\n",
            session.branch_name, project.branch_dev, merge_code,
            merge_err.chars().take(2000).collect::<String>()
        );
        let _ = sqlx::query(
            "UPDATE worktree_sessions SET report_content=? WHERE id=?",
        )
        .bind(&fail_reason)
        .bind(&session.id)
        .execute(db)
        .await;
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

    // Note: tests already ran as a pre-merge gate above, so there is no
    // post-merge testing job to enqueue here.

    // Node 07: run a security audit on the merged diff.
    let _ = enqueue(
        db,
        tx,
        "security_audit",
        &format!("security_audit:{}", cr_id),
        JobPayload::SecurityAudit {
            change_request_id: cr_id.to_string(),
        },
    )
    .await;

    crate::core::notify::dispatch(db, "cr_merged", "已合并到 dev", cr_id).await;

    // Innate: capture the merged implementation as a SUCCESS exemplar (positive signal) —
    // 它已通过审核 2 + 测试并合并，是"这类需求该怎么改"的高质量样本，供需求分析/代码实现角色召回。
    let issue_title: String = sqlx::query_as::<_, (String,)>("SELECT title FROM issues WHERE id=?")
        .bind(&cr.issue_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|t| t.0)
        .unwrap_or_default();
    if let Some(report) = session.report_content.as_deref().filter(|r| !r.trim().is_empty()) {
        let content = format!(
            "已合并需求「{}」的成功实现方案（通过审核 2 与测试）：\n\n{}",
            issue_title, report
        );
        let trigger = format!("实现该项目同类需求时可参考的成功改动方案；相关需求：{}", issue_title);
        crate::knowledge::kb_add(&cr.project_id, &content, &trigger).await;
    }

    // Innate: close the recall feedback loop — the recalled knowledge fed code
    // that passed review 2 + tests and merged, so reinforce it (positive signal).
    crate::knowledge::consume_trace_outcome(db, cr_id, "ok", Some("up")).await;

    // Innate: distil this project's accumulated logs into knowledge after a successful merge.
    crate::knowledge::kb_evolve(&cr.project_id).await;

    info!("merge completed for cr {}", cr_id);
    Ok(())
}
