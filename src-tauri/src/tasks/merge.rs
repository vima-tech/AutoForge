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
    merge_msg: &str,
) -> std::result::Result<String, (i32, String)> {

    if !dev_is_live {
        // Fast path (all non-self-managed projects): in-place checkout + merge.
        let (cc, _, ce) = git
            .run(&["checkout", &project.branch_dev])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if cc != 0 {
            return Err((cc, format!("checkout {} 失败：{}", project.branch_dev, ce)));
        }
        // Clear any half-applied squash left staged on dev by a previous attempt that
        // crashed between `merge --squash`(stages) and `commit`. `--squash` sets no
        // MERGE_HEAD, so `merge --abort` can't undo it; the next `merge --squash` below
        // refuses to run against a dirty index ("local changes would be overwritten") and
        // would spuriously fail the recovered merge. `reset --hard HEAD` discards only that
        // uncommitted residue — committed dev history is untouched — making this re-entrant.
        // Safe because a successful `checkout dev` already implies a clean-enough tree.
        let _ = git.run(&["reset", "--hard", "HEAD"]).await;
        // Squash-merge: collapse the CR branch (impl commits + Phase-1 dev-sync
        // merge) into a SINGLE commit on dev, so a batch of N CRs leaves N commits
        // instead of ~3N. Phase 1 already merged dev into the branch, so this
        // squash applies only the CR's own changes (dev parts are already in dev).
        let (mc, _, me) = git
            .run(&["merge", "--squash", &session.branch_name])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if mc != 0 {
            // --squash never sets MERGE_HEAD, so `merge --abort` can't undo a
            // conflicted squash; reset hard to restore dev for a retry.
            let _ = git.run(&["reset", "--hard", "HEAD"]).await;
            return Err((mc, me));
        }
        // Branch already fully contained in dev (e.g. its change landed via an
        // earlier CR in the same batch) ⇒ squash stages nothing. `git commit` would
        // fail with "nothing to commit"; `--no-ff` used to report "Already up to
        // date" and succeed here, so mirror that — succeed WITHOUT an empty commit.
        let nothing_staged = git
            .run(&["diff", "--cached", "--quiet"])
            .await
            .map(|(c, _, _)| c == 0)
            .unwrap_or(false);
        if nothing_staged {
            // No CR-specific commit produced ⇒ nothing to revert later. Empty SHA.
            return Ok(String::new());
        }
        let (cc2, _, ce2) = git
            .run(&[
                "-c",
                "user.name=AutoForge",
                "-c",
                "user.email=autoforge@local",
                "commit",
                "-m",
                merge_msg,
            ])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if cc2 != 0 {
            let _ = git.run(&["reset", "--hard", "HEAD"]).await;
            return Err((cc2, ce2));
        }
        // Capture the squash commit SHA — this is the unit `tasks/revert.rs` reverts.
        return match git.run(&["rev-parse", "HEAD"]).await {
            Ok((0, out, _)) => Ok(out.trim().to_string()),
            Ok((c, _, e)) => Err((c, e)),
            Err(e) => Err((-1, e.to_string())),
        };
    }

    // Isolated path (AutoForge self-managed: dev is checked out live).
    // No fetch here: Phase 1 (sync_dev_into_worktree) already `git fetch`ed
    // origin/<dev> into the SHARED object store/refs (a worktree shares the main
    // repo's remote-tracking refs), so origin/<dev> is already fresh — a second
    // fetch is a redundant network round-trip held under the merge lock. Offline
    // still falls back to local dev below, exactly as before.
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
    // Squash-merge into the throwaway branch (same rationale as the fast path):
    // the push lands ONE commit on origin/<dev> per CR, not the branch's full history.
    let (mc, _, me) = tmp_git
        .run(&["merge", "--squash", &session.branch_name])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if mc != 0 {
        // --squash sets no MERGE_HEAD; reset hard to restore the tmp branch.
        let _ = tmp_git.run(&["reset", "--hard", "HEAD"]).await;
        let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
        let _ = git.run(&["branch", "-D", &tmp_branch]).await;
        return Err((mc, me));
    }
    // Nothing staged ⇒ already in origin/<dev>; skip the (empty) commit and let the
    // push below be a no-op fast-forward, mirroring `--no-ff`'s "Already up to date".
    let nothing_staged = tmp_git
        .run(&["diff", "--cached", "--quiet"])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);
    if !nothing_staged {
        let (cc2, _, ce2) = tmp_git
            .run(&[
                "-c",
                "user.name=AutoForge",
                "-c",
                "user.email=autoforge@local",
                "commit",
                "-m",
                merge_msg,
            ])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if cc2 != 0 {
            let _ = tmp_git.run(&["reset", "--hard", "HEAD"]).await;
            let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
            let _ = git.run(&["branch", "-D", &tmp_branch]).await;
            return Err((cc2, ce2));
        }
    }

    // Capture the squash commit SHA before teardown (empty when nothing staged ⇒ no
    // CR-specific commit). This is what `tasks/revert.rs` reverts on origin/<dev>.
    let merge_sha = if nothing_staged {
        String::new()
    } else {
        match tmp_git.run(&["rev-parse", "HEAD"]).await {
            Ok((0, out, _)) => out.trim().to_string(),
            _ => String::new(),
        }
    };

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
    Ok(merge_sha)
}

/// Result of bringing `dev` into the CR worktree branch before landing.
enum DevSync {
    /// Merge applied cleanly. `dev_merged` is true when it actually integrated new
    /// dev commits (HEAD moved), false when the branch was already up to date.
    Clean { dev_merged: bool },
    /// `git merge` hit textual conflicts; worktree restored via `merge --abort`.
    Conflict { files: Vec<String>, diff: String },
}

/// Phase 1: merge the latest `dev` into the CR's worktree branch so (a) the
/// pre-merge tests below run on the INTEGRATED result and (b) the final
/// `land_on_dev` is a conflict-free merge. Uses `rerere` to reuse past
/// resolutions. Never touches the project's live working tree — it operates only
/// inside the isolated CR worktree, so this is safe regardless of `dev_is_live`.
async fn sync_dev_into_worktree(
    worktree_path: &str,
    branch_name: &str,
    branch_dev: &str,
) -> DevSync {
    if !std::path::Path::new(worktree_path).exists() {
        return DevSync::Clean { dev_merged: false };
    }
    let wt = GitProxy::new(worktree_path);
    // Clear any stale in-progress merge left by opening the conflict resolver
    // (get_conflict_detail / open_conflict_workspace materialize a merge and may not be
    // followed by a resolve). No-op when not mid-merge; safe because we re-merge dev next.
    let _ = wt.run(&["merge", "--abort"]).await;
    let _ = wt.run(&["config", "rerere.enabled", "true"]).await;
    let _ = wt.run(&["fetch", "origin", branch_dev]).await;
    // Prefer origin/<dev> so we integrate the newest dev (matches land_on_dev's base).
    let remote_ref = format!("origin/{}", branch_dev);
    let dev_ref = if wt
        .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false)
    {
        remote_ref
    } else {
        branch_dev.to_string()
    };
    let before = wt
        .run(&["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|(_, o, _)| o.trim().to_string());
    let merge_msg = format!("AutoForge: sync {} into {}", dev_ref, branch_name);
    let (code, _, _) = wt
        .run(&["merge", "-m", &merge_msg, &dev_ref])
        .await
        .unwrap_or((-1, String::new(), String::new()));
    if code == 0 {
        let after = wt
            .run(&["rev-parse", "HEAD"])
            .await
            .ok()
            .map(|(_, o, _)| o.trim().to_string());
        return DevSync::Clean {
            dev_merged: before != after,
        };
    }
    // Non-zero: conflict iff there are unmerged paths. Anything else (git missing,
    // nothing to merge) is treated as clean so an infra hiccup never blocks merge.
    let files: Vec<String> = wt
        .run(&["diff", "--name-only", "--diff-filter=U"])
        .await
        .ok()
        .filter(|(c, _, _)| *c == 0)
        .map(|(_, o, _)| {
            o.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        let _ = wt.run(&["merge", "--abort"]).await;
        return DevSync::Clean { dev_merged: false };
    }
    // Capture the conflict hunks (markers) BEFORE aborting restores the tree.
    let diff = wt
        .run(&["diff"])
        .await
        .ok()
        .map(|(_, o, _)| o)
        .unwrap_or_default();
    let _ = wt.run(&["merge", "--abort"]).await;
    DevSync::Conflict { files, diff }
}

/// 解析用于 merge 的 dev 引用：优先 `origin/<dev>`（与 land_on_dev 一致），不可达回退本地。
pub(crate) async fn resolve_dev_ref(wt: &GitProxy, branch_dev: &str) -> String {
    let remote_ref = format!("origin/{}", branch_dev);
    if wt
        .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false)
    {
        remote_ref
    } else {
        branch_dev.to_string()
    }
}

/// worktree 内当前未合并（冲突）的文件列表。
pub(crate) async fn list_unmerged(wt: &GitProxy) -> Vec<String> {
    wt.run(&["diff", "--name-only", "--diff-filter=U"])
        .await
        .ok()
        .map(|(_, o, _)| {
            o.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 把 CR worktree 置于「与 dev 冲突的 MERGE 进行态」——若上一步 abort 过则重新合并重建。
/// 幂等：已处于 MERGE 中（存在 MERGE_HEAD）则原样保留，重复打开解决器不重跑。开启 rerere。
/// 返回所用的 dev 引用。供 AI 自动解冲突与人工/外部解冲突共用同一现场。
pub(crate) async fn materialize_conflict(wt: &GitProxy, branch_name: &str, branch_dev: &str) -> String {
    let _ = wt.run(&["config", "rerere.enabled", "true"]).await;
    let _ = wt.run(&["fetch", "origin", branch_dev]).await;
    let dev_ref = resolve_dev_ref(wt, branch_dev).await;
    let in_merge = wt
        .run(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);
    if !in_merge {
        let merge_msg = format!("AutoForge: sync {} into {}", dev_ref, branch_name);
        let _ = wt.run(&["merge", "-m", &merge_msg, &dev_ref]).await;
    }
    dev_ref
}

/// 三路共享的「解冲突收尾」：暂存 worktree → 校验冲突标记已全清 → 提交 → 复跑测试门 →
/// 通过【回到代码审核】复审、失败置 `merge_failed`。绝不直接落 dev。AI 自动解、应用内逐
/// hunk 决策、外部 IDE 解决都收敛到这里。返回 `Ok(true)` 表示已回代码审核，`Ok(false)`
/// 表示退回 `merge_conflict`/`merge_failed`。
pub(crate) async fn finalize_resolution(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    issue: &crate::models::issue::Issue,
    commit_msg: &str,
) -> Result<bool> {
    let wt = GitProxy::new(&session.worktree_path);
    // Stage everything and verify there are no leftover conflict markers / unmerged paths.
    let _ = wt.run(&["add", "-A"]).await;
    let markers_clean = wt
        .run(&["diff", "--cached", "--check"])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);
    let still_unmerged = !list_unmerged(&wt).await.is_empty();
    if !markers_clean || still_unmerged {
        let _ = wt.run(&["merge", "--abort"]).await;
        set_status(db, &cr.id, &cr.issue_id, "merge_conflict").await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr.id.clone(),
                status: "merge_conflict".to_string(),
                message: Some("仍有未解决的冲突标记，需继续处理".to_string()),
            },
        );
        return Ok(false);
    }

    let _ = wt
        .run(&[
            "-c",
            "user.name=AutoForge",
            "-c",
            "user.email=autoforge@local",
            "commit",
            "-m",
            commit_msg,
        ])
        .await;
    let _ = sqlx::query("UPDATE worktree_sessions SET conflict_files=NULL, conflict_diff=NULL WHERE id=?")
        .bind(&session.id)
        .execute(db)
        .await;

    // Re-run the test gate on the integrated result. Failure blocks (merge_failed).
    let passed = crate::tasks::testing::run_and_gate(db, tx, app, &cr.id)
        .await
        .unwrap_or(false);
    if !passed {
        // 把测试失败详情回写报告，供审核页「合并失败原因」展示有用信息（而非实现摘要）。
        crate::tasks::testing::persist_test_failure_report(db, &cr.id).await;
        set_status(db, &cr.id, &cr.issue_id, "merge_failed").await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr.id.clone(),
                status: "merge_failed".to_string(),
                message: Some("解冲突后测试未通过，已阻断合并".to_string()),
            },
        );
        return Ok(false);
    }

    // Route back to human review 2 — never land a resolved conflict directly.
    set_status(db, &cr.id, &cr.issue_id, "pending_code_review").await?;
    crate::core::notify::dispatch(db, "review_needed", &issue.title, "合并冲突已解决，待代码审核 复审").await;
    event::emit(
        app,
        event::AppEvent::ReviewNeeded {
            cr_id: cr.id.clone(),
            issue_title: issue.title.clone(),
            stage: 2,
        },
    );
    info!("conflict resolved for cr {}, routed back to review 2", cr.id);
    Ok(true)
}

/// 方案 B：把合并冲突交给 code agent 自动消解，复跑测试后【回到代码审核】复审，
/// 绝不直接落 dev。手动「AI 解冲突并合并」按钮与 Phase 1 自动开关共用此函数。
pub async fn ai_resolve_conflict(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    cr_id: &str,
) -> Result<()> {
    // Serialize with merge::run and any other resolve for this CR (H2: no concurrent
    // git on the same worktree). Held for the whole resolve.
    let cr_lock = crate::state::cr_lock(cr_id);
    let _cr_guard = cr_lock.lock().await;
    // Re-check under the lock: a prior resolve (auto-spawn vs. manual button) may have
    // already moved this CR off merge_conflict. If so, this invocation is a duplicate
    // (the lock just serialized us behind the winner) — no-op instead of re-merging.
    let cur_status = sqlx::query_as::<_, (String,)>("SELECT status FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
        .await?
        .map(|(s,)| s);
    if cur_status.as_deref() != Some("merge_conflict") {
        info!(
            "ai_resolve_conflict: cr {} no longer in conflict ({:?}); skipping duplicate resolve",
            cr_id, cur_status
        );
        return Ok(());
    }
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("no worktree session for cr {}", cr_id))?;
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
    let issue =
        sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&cr.issue_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("issue {} not found", cr.issue_id))?;

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "resolving_conflict".to_string(),
            note: Some("AI 正在解决合并冲突…".to_string()),
        },
    );

    let wt = GitProxy::new(&session.worktree_path);
    // Recreate the conflict现场 (Phase 1 aborted it); rerere may auto-resolve some hunks.
    let dev_ref = materialize_conflict(&wt, &session.branch_name, &project.branch_dev).await;
    let unmerged = list_unmerged(&wt).await;

    if !unmerged.is_empty() {
        let conflict_view = wt
            .run(&["diff"])
            .await
            .ok()
            .map(|(_, o, _)| o)
            .unwrap_or_default();
        let prompt = format!(
            "你在一个 git worktree 里，刚把 `{dev}` 合并进当前分支 `{br}` 时发生了代码冲突。\n\
             需求标题：{title}\n需求描述：{desc}\n\n\
             请打开下列冲突文件，逐处消除冲突标记（<<<<<<< ======= >>>>>>>），\
             同时保留【本分支新增功能】与【dev 上其他改动】两边的意图，确保代码逻辑正确、可编译：\n{files}\n\n\
             带冲突标记的当前 diff 供参考：\n```\n{diff}\n```\n\n\
             只修改这些文件以解决冲突，不要改动无关代码，不要执行任何 git 命令。",
            dev = dev_ref,
            br = session.branch_name,
            title = issue.title,
            desc = issue.description,
            files = unmerged
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n"),
            diff = conflict_view.chars().take(12000).collect::<String>(),
        );
        let code_agent = crate::agents::code_agent::resolve(db, &project).await;
        // 解冲突上限 10 分钟，空闲 5 分钟无输出即判卡死并真杀进程组。
        let limits = crate::agents::code_agent::RunLimits {
            wall_secs: 600,
            idle_secs: 300,
        };
        // 解冲突同属编码任务：同样注入「适用于编码 Agent」的 MCP（pull）+ 技能（skill）。
        let code_mcp = crate::agents::code_agent::load_code_agent_mcp(db).await;
        let code_skills = crate::agents::code_agent::load_code_agent_skills(db, &project).await;
        let run_started = std::time::Instant::now();
        // 实时日志：解冲突也推 CodeAgentLog（phase=conflict_resolve），前端同样可实时滚动。
        let (log_tx, mut log_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::agents::code_agent::LogChunk>();
        let forward = {
            let app = app.clone();
            let cr = cr_id.to_string();
            tokio::spawn(async move {
                while let Some(c) = log_rx.recv().await {
                    let seq = crate::state::running_log_append(&cr, &c.text);
                    crate::core::event::emit(
                        &app,
                        crate::core::event::AppEvent::CodeAgentLog {
                            cr_id: cr.clone(),
                            phase: "conflict_resolve".to_string(),
                            stream: c.stream.to_string(),
                            chunk: c.text,
                            seq,
                        },
                    );
                }
            })
        };
        let (_code, report, _err) = code_agent
            .run(&session.worktree_path, &prompt, limits, &code_mcp, &code_skills, Some(&log_tx))
            .await
            .unwrap_or((-1, String::new(), String::new()));
        drop(log_tx);
        let _ = forward.await;
        crate::state::running_log_clear(cr_id); // 清理实时缓冲（落库列表接管）
        // 解冲突也是一次代码 Agent 执行，完整日志同样落库（phase=conflict_resolve）。
        crate::agents::code_agent::log_run(
            db,
            crate::agents::code_agent::RunLogInput {
                change_request_id: cr_id,
                worktree_session_id: Some(&session.id),
                phase: "conflict_resolve",
                kind: code_agent.kind(),
                model: None,
                exit_code: _code,
                stdout: &report,
                stderr: &_err,
                duration_ms: run_started.elapsed().as_millis() as i64,
            },
        )
        .await;
        // agent 输出视为外部输入：留档/回灌前过注入检测（命中只记录，文件改动才是结果）。
        if crate::core::security::has_obvious_injection(&report) {
            info!(
                "AI conflict-resolve report for {} tripped injection filter",
                cr_id
            );
        }
    }

    // Shared tail: stage / verify markers gone / commit / re-test / route to review 2.
    let commit_msg = format!("AutoForge: AI 解决合并冲突（{} → {}）", dev_ref, session.branch_name);
    finalize_resolution(db, tx, app, &session, &cr, &issue, &commit_msg)
        .await
        .map(|_| ())
}

/// 把需求 category 映射到 Conventional Commits 前缀（feat/fix/docs…）。
fn commit_prefix_for_category(category: &str) -> &'static str {
    match category.trim().to_ascii_lowercase().as_str() {
        "feature" | "feat" => "feat",
        "bug" | "fix" => "fix",
        "improvement" | "refactor" | "perf" => "refactor",
        "debt" | "chore" => "chore",
        "docs" | "documentation" => "docs",
        _ => "chore",
    }
}

/// 构造合并提交信息的默认模板：`<前缀>(<修改模块>): <需求标题> [autoforge #<需求编号>]`。
///
/// 前缀取自需求 category（Feature→feat / Bug→fix / Improvement→refactor / Debt→chore…），
/// 修改模块取自最近一次需求分析的 affected_modules（空则省略括号段），需求编号沿用 UI 口径
/// （issue id 前 8 位短码）。任何字段缺失时优雅降级，最差回退到旧模板，保证永远产出非空信息。
pub async fn default_merge_message(db: &Db, cr: &crate::models::change_request::ChangeRequest) -> String {
    let issue = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&cr.issue_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();

    let Some(issue) = issue else {
        // 需求不存在（理论上不会发生）——回退旧模板，绝不产出空提交信息。
        return format!("AutoForge merge: {}", cr.id);
    };

    let prefix = commit_prefix_for_category(&issue.category);

    // 修改模块：取最近一次分析的 affected_modules（JSON 数组），最多取 2 个用「/」连接。
    let modules = sqlx::query_scalar::<_, Option<String>>(
        "SELECT affected_modules FROM issue_analyses WHERE issue_id=? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&issue.id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .flatten()
    .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
    .map(|mods| {
        mods.into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .take(2)
            .collect::<Vec<_>>()
            .join("/")
    })
    .filter(|m| !m.is_empty());

    // 需求标题：折叠空白、去首尾，超长截断（避免单行提交信息过长）。
    let mut title = issue.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > 60 {
        title = title.chars().take(60).collect::<String>() + "…";
    }
    if title.is_empty() {
        title = "需求变更".to_string();
    }

    // 需求编号：与审核页一致，取 issue id 前 8 位短码。
    let short_id: String = issue.id.chars().take(8).collect();

    let scope = modules.map(|m| format!("({})", m)).unwrap_or_default();
    format!("{}{}: {} [autoforge #{}]", prefix, scope, title, short_id)
}

/// 合并流水线统一入口（review_2 批准 / 自动合并门 / 重试合并共用）。
/// 开关 ON → 入队并行 `premerge`（测试出锁）；OFF → 入队 legacy `merge`（单锁全流程）。
/// 一律用带 uuid 的唯一键：premerge/merge job 均幂等，唯一键杜绝撞历史 completed 行被
/// `enqueue` 去重而不派发（CR 卡死 pending_merge 的根因）。`reason` 仅供幂等键可读。
pub async fn enqueue_merge_pipeline(db: &Db, tx: &JobSender, cr_id: &str, reason: &str) {
    let (job, payload) = if crate::core::gate::parallel_premerge_enabled(db).await {
        (
            "premerge",
            JobPayload::Premerge {
                change_request_id: cr_id.to_string(),
            },
        )
    } else {
        (
            "merge",
            JobPayload::Merge {
                change_request_id: cr_id.to_string(),
            },
        )
    };
    let key = format!("{}:{}:{}:{}", job, cr_id, reason, uuid::Uuid::new_v4());
    let _ = enqueue(db, tx, job, &key, payload).await;
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

    // L2：worktree 缺失时绝不继续——否则 run_and_gate 会回退到主仓库路径（testing.rs），
    // 在【错误的树】上跑测试却仍尝试 land 该分支。置 merge_failed 引导重新执行重建 worktree。
    if !std::path::Path::new(&session.worktree_path).exists() {
        let msg = "CR worktree 不存在，无法在正确的分支上测试/合并；请「重新执行」基于最新代码重建后再合并。";
        let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
            .bind(msg)
            .bind(&session.id)
            .execute(db)
            .await;
        set_status(db, cr_id, &cr.issue_id, "merge_failed").await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_failed".to_string(),
                message: Some(msg.to_string()),
            },
        );
        return Ok(());
    }

    // ── Phase 0：同项目合并串行 ─────────────────────────────────────────────
    // 全程持有该项目的合并锁，避免并发 `checkout dev && merge` 互相踩同一个 dev
    // 工作树（竞态）；跨项目仍并行。锁在函数返回时随 guard 释放。
    let merge_lock = crate::state::merge_lock(&cr.project_id);
    let _merge_guard = merge_lock.lock().await;
    // Also hold this CR's worktree lock so a concurrent conflict-resolve (auto/manual)
    // can't write the same worktree underneath us. Order: merge_lock → cr_lock (no deadlock).
    let cr_lock = crate::state::cr_lock(cr_id);
    let _cr_guard = cr_lock.lock().await;

    // 串行合并路径此前全程停在 pending_merge（「待合并」）直到翻成 merged——用户看不到「正在
    // 合并」。这里一进入实际合并就推进到 merge_testing（「合并中」），与并行 premerge 路径一致；
    // 启动恢复已覆盖 merge_testing（重启会重排 premerge），故崩溃也不会卡死。发 WorktreeUpdate
    // 触发审核页列表重载、让状态 chip 立刻翻成「合并中」（task_progress 只更新进度 note，不刷状态）。
    set_status(db, cr_id, &cr.issue_id, "merge_testing").await?;
    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "merge_testing".to_string(),
            message: Some(format!("正在合并到 {}…", project.branch_dev)),
        },
    );

    // Snapshot the CR's own diff before Phase 1 merges dev in (keeps it scoped).
    snapshot_cr_diff(db, &session, &cr).await;

    // Phase 1 dev-sync + 测试门 + 安全门（仅碰 CR worktree）。任一失败已置态 → 直接返回。
    match run_premerge_gates(db, tx, app, &session, &cr, &project, cr_id).await? {
        GateResult::Stopped => return Ok(()),
        GateResult::Pass { .. } => {}
    }

    // Land on dev（持锁内，串行）。legacy 单锁路径：gate 与 land 同处一把 merge_lock 下。
    land_core(db, tx, app, &session, &cr, &project, cr_id).await
}

/// 合并前阶段（并行）：仅持 `cr_lock`（防解冲突并发写同一 worktree），**不持 merge_lock**，
/// 故同项目多个 CR 的 premerge 可并行（受 `build_pool` 节流）。跑 Phase1 dev-sync + 测试门 +
/// 安全门；通过则落 `tested_dev_sha` + 置 `merge_ready` 并入队 Merge(land)。开关关闭时兜底走
/// legacy `run()`（自带 merge_lock，行为不变）。
pub async fn premerge_run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    if !crate::core::gate::parallel_premerge_enabled(db).await {
        return run(db, tx, app, cr_id).await;
    }

    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("no worktree session found for cr {}", cr_id))?;
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

    if !std::path::Path::new(&session.worktree_path).exists() {
        return fail_missing_worktree(db, app, &session, &cr, cr_id).await;
    }

    // cr_lock ONLY — no merge_lock, so sibling CRs premerge in parallel.
    let cr_lock = crate::state::cr_lock(cr_id);
    let _cr_guard = cr_lock.lock().await;

    set_status(db, cr_id, &cr.issue_id, "merge_testing").await?;
    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "premerge".to_string(),
            note: Some("合并前测试中（并行）…".to_string()),
        },
    );

    snapshot_cr_diff(db, &session, &cr).await;

    let tested_dev_sha = match run_premerge_gates(db, tx, app, &session, &cr, &project, cr_id).await? {
        GateResult::Stopped => return Ok(()),
        GateResult::Pass { tested_dev_sha } => tested_dev_sha,
    };

    // 通过：落 tested_dev_sha + premerge_at，置 merge_ready，入队 land。
    let _ = sqlx::query(
        "UPDATE worktree_sessions SET tested_dev_sha=?, premerge_at=datetime('now') WHERE id=?",
    )
    .bind(&tested_dev_sha)
    .bind(&session.id)
    .execute(db)
    .await;
    set_status(db, cr_id, &cr.issue_id, "merge_ready").await?;
    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "merge_ready".to_string(),
            note: Some("已通过测试，待落地 dev".to_string()),
        },
    );
    // Unique key：land job 幂等，唯一键杜绝撞历史 completed 行被去重不派发。
    let _ = enqueue(
        db,
        tx,
        "merge",
        &format!("land:{}:{}", cr_id, uuid::Uuid::new_v4()),
        JobPayload::Merge {
            change_request_id: cr_id.to_string(),
        },
    )
    .await;
    info!("premerge passed for cr {}, queued land", cr_id);
    Ok(())
}

/// 落地阶段（串行）：持 `merge_lock` + `cr_lock`。再校验 dev 是否在「测后落地前」前进（§5），
/// 据此直落 / 独立直落 / 重叠补测 / 冲突回退；通过则 `land_core` 落地。只在 CR 已 premerge
/// （`tested_dev_sha` 存在、状态 `merge_ready`）时由派发表路由到此。
pub async fn land_run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
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

    // merge_lock → cr_lock（固定顺序，无死锁）。land 廉价，锁只覆盖再校验 + 落地。
    let merge_lock = crate::state::merge_lock(&cr.project_id);
    let _merge_guard = merge_lock.lock().await;
    let cr_lock = crate::state::cr_lock(cr_id);
    let _cr_guard = cr_lock.lock().await;

    // 锁内复读最新状态：兄弟 land / 重启恢复可能已推进它。只从 merge_ready 落地。
    let status: Option<String> =
        sqlx::query_as::<_, (String,)>("SELECT status FROM change_requests WHERE id=?")
            .bind(cr_id)
            .fetch_optional(db)
            .await?
            .map(|(s,)| s);
    if status.as_deref() != Some("merge_ready") {
        info!("land_run: cr {} not in merge_ready ({:?}); skip", cr_id, status);
        return Ok(());
    }

    // 复读 session（premerge 刚写了 tested_dev_sha）。
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("no worktree session for cr {}", cr_id))?;

    if !std::path::Path::new(&session.worktree_path).exists() {
        return fail_missing_worktree(db, app, &session, &cr, cr_id).await;
    }

    match revalidate(db, tx, app, &session, &cr, &project, cr_id).await? {
        Revalidate::Land { verify } => {
            land_core(db, tx, app, &session, &cr, &project, cr_id).await?;
            // Phase 3 lite：独立改动免重测直落 → 落地后补一发合并后集成测试（在 dev 上跑，
            // worktree 已删故 run_and_gate 回退到仓库路径=dev）。失败由现有机制登记 bug 需求
            // 告警，不自动 revert。无 merge_lock，不阻塞后续 land。
            if verify {
                let _ = enqueue(
                    db,
                    tx,
                    "testing",
                    &format!("postmerge_verify:{}:{}", cr_id, uuid::Uuid::new_v4()),
                    JobPayload::Testing {
                        change_request_id: cr_id.to_string(),
                    },
                )
                .await;
                info!("land_run: enqueued post-merge integration verify for cr {}", cr_id);
            }
            Ok(())
        }
        Revalidate::Requeued | Revalidate::Conflict => Ok(()),
    }
}

/// 派发路由用：CR 是否已走完 premerge（落了 `tested_dev_sha`）→ 走 land_run；否则 legacy run。
pub async fn should_land_only(db: &Db, cr_id: &str) -> bool {
    sqlx::query_as::<_, (Option<String>,)>(
        "SELECT tested_dev_sha FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(s,)| s.is_some())
    .unwrap_or(false)
}

/// land 阶段再校验结果。
enum Revalidate {
    /// dev 未动或独立改动（已干净 re-sync）→ 可直接落地。
    /// `verify`=true 表示是「独立改动免重测」直落（集成态未被整体测过）→ 落地后补一发
    /// 合并后集成测试作为安全网（Phase 3 lite：失败经现有测试机制登记为 bug 需求告警，
    /// 不做危险的自动 revert）。`verify`=false 表示落的就是 premerge 测过的精确组合，无需补测。
    Land { verify: bool },
    /// 重叠改动、re-sync 干净 → 已重排 premerge 补测，调用方返回。
    Requeued,
    /// re-sync 冲突 → 已置 merge_conflict（可选 AI 解冲突），调用方返回。
    Conflict,
}

/// §5 决策树：比对 premerge 时的 `tested_dev_sha` 与当前 dev tip。
async fn revalidate(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    project: &crate::models::project::Project,
    cr_id: &str,
) -> Result<Revalidate> {
    let wt = GitProxy::new(&session.worktree_path);
    let _ = wt.run(&["fetch", "origin", &project.branch_dev]).await;
    let dev_ref = resolve_dev_ref(&wt, &project.branch_dev).await;
    let dev_now = wt
        .run(&["rev-parse", &dev_ref])
        .await
        .ok()
        .filter(|(c, _, _)| *c == 0)
        .map(|(_, o, _)| o.trim().to_string())
        .filter(|s| !s.is_empty());
    let tested = session.tested_dev_sha.clone();

    // ① dev 未动：落的就是测过的组合，零额外成本，无需合并后补测。
    if let (Some(t), Some(n)) = (&tested, &dev_now) {
        if t == n {
            return Ok(Revalidate::Land { verify: false });
        }
    }

    // dev 前进了：判断兄弟 CR 的改动是否与本 CR 文件相交。
    let disjoint = match (&tested, &dev_now) {
        (Some(t), Some(n)) => {
            let moved = diff_name_only(&wt, t, n).await;
            let cr_files = parse_changeset_files(session.diff_content.as_deref());
            !cr_files.is_empty() && moved.is_disjoint(&cr_files)
        }
        _ => false,
    };

    // Re-sync 最新 dev 进分支，使 land 必为干净 squash。
    match sync_dev_into_worktree(&session.worktree_path, &session.branch_name, &project.branch_dev)
        .await
    {
        DevSync::Conflict { files, diff } => {
            let files_json = serde_json::to_string(&files).unwrap_or_else(|_| "[]".into());
            let report = format!(
                "## 合并冲突\n\n落地前再次并入 `{}` 到 `{}` 时发生冲突（{} 个文件）：\n\n{}\n\n可在审核页三方解决、一键重试，或交由 AI 自动解冲突。",
                project.branch_dev,
                session.branch_name,
                files.len(),
                files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
            );
            let _ = sqlx::query(
                "UPDATE worktree_sessions SET conflict_files=?, conflict_diff=?, report_content=?, tested_dev_sha=NULL WHERE id=?",
            )
            .bind(&files_json)
            .bind(&diff)
            .bind(&report)
            .bind(&session.id)
            .execute(db)
            .await;
            set_status(db, cr_id, &cr.issue_id, "merge_conflict").await?;
            event::emit(
                app,
                event::AppEvent::MergeConflict {
                    cr_id: cr_id.to_string(),
                    files: files.clone(),
                },
            );
            info!("revalidate dev-sync conflict for cr {} ({} files)", cr_id, files.len());
            if crate::core::gate::auto_conflict_resolve_enabled(db).await {
                let (db2, tx2, app2, cr2) = (db.clone(), tx.clone(), app.clone(), cr_id.to_string());
                tokio::spawn(async move {
                    if let Err(e) = ai_resolve_conflict(&db2, &tx2, &app2, &cr2).await {
                        info!("AI conflict-resolve failed for {}: {}", cr2, e);
                    }
                });
            }
            Ok(Revalidate::Conflict)
        }
        DevSync::Clean { .. } => {
            if disjoint {
                // 独立改动：兄弟 CR 落地的文件与本 CR 不相交，premerge 测试结论仍成立 → 直落。
                // verify=true：落地后补一发集成测试做安全网（文件不相交≠语义兼容）。
                info!("revalidate: cr {} independent of advanced dev, landing without retest", cr_id);
                Ok(Revalidate::Land { verify: true })
            } else {
                // 重叠（或无 diff 信息）：必须在集成后的代码上重测 → 退回 premerge（锁外重测）。
                info!("revalidate: cr {} overlaps advanced dev, re-queuing premerge for retest", cr_id);
                let _ = sqlx::query(
                    "UPDATE worktree_sessions SET tested_dev_sha=NULL WHERE id=?",
                )
                .bind(&session.id)
                .execute(db)
                .await;
                set_status(db, cr_id, &cr.issue_id, "merge_testing").await?;
                let _ = enqueue(
                    db,
                    tx,
                    "premerge",
                    &format!("premerge:{}:revalidate:{}", cr_id, uuid::Uuid::new_v4()),
                    JobPayload::Premerge {
                        change_request_id: cr_id.to_string(),
                    },
                )
                .await;
                Ok(Revalidate::Requeued)
            }
        }
    }
}

/// `git diff --name-only a..b` → 文件名集合。
async fn diff_name_only(wt: &GitProxy, a: &str, b: &str) -> std::collections::HashSet<String> {
    wt.run(&["diff", "--name-only", &format!("{}..{}", a, b)])
        .await
        .ok()
        .filter(|(c, _, _)| *c == 0)
        .map(|(_, o, _)| {
            o.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 从快照的统一 diff 中解析本 CR 改动的文件集（`diff --git a/<path> b/<path>` 行）。
fn parse_changeset_files(diff: Option<&str>) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Some(d) = diff else {
        return set;
    };
    for line in d.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            if let Some(idx) = rest.find(" b/") {
                let p = rest[..idx].trim();
                if !p.is_empty() {
                    set.insert(p.to_string());
                }
            }
        }
    }
    set
}

/// 把 dev 并入分支前快照本 CR 相对分叉点的 diff（worktree 删除后审核页据此回看）。
async fn snapshot_cr_diff(
    db: &Db,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
) {
    if let Some(diff) = crate::commands::change_requests::compute_worktree_diff(
        &session.worktree_path,
        &session.branch_name,
        &cr.target_branch,
        session.base_commit.as_deref(),
    )
    .await
    {
        if !diff.is_empty() {
            let _ = sqlx::query("UPDATE worktree_sessions SET diff_content=? WHERE id=?")
                .bind(&diff)
                .bind(&session.id)
                .execute(db)
                .await;
        }
    }
}

/// 同时把 CR 与其**全部成员需求**置为同一状态（合并流水线全程保持一致）。
/// 合并 CR（一个 CR 覆盖多个需求）下全组联动；单需求 CR 等价于只动那一条。
/// issue_id 参数保留为兜底（关联表缺失时仍同步主需求），真源是 change_request_issues。
async fn set_status(db: &Db, cr_id: &str, issue_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE change_requests SET status=?, updated_at=datetime('now') WHERE id=?")
        .bind(status)
        .bind(cr_id)
        .execute(db)
        .await?;
    sqlx::query(
        "UPDATE issues SET status=?, updated_at=datetime('now')
         WHERE id IN (SELECT issue_id FROM change_request_issues WHERE change_request_id=?)
            OR id = ?",
    )
    .bind(status)
    .bind(cr_id)
    .bind(issue_id)
    .execute(db)
    .await?;
    Ok(())
}

/// worktree 缺失的统一收尾：置 merge_failed + 引导重新执行。
async fn fail_missing_worktree(
    db: &Db,
    app: &tauri::AppHandle,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    cr_id: &str,
) -> Result<()> {
    let msg = "CR worktree 不存在，无法在正确的分支上测试/合并；请「重新执行」基于最新代码重建后再合并。";
    let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
        .bind(msg)
        .bind(&session.id)
        .execute(db)
        .await;
    set_status(db, cr_id, &cr.issue_id, "merge_failed").await?;
    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "merge_failed".to_string(),
            message: Some(msg.to_string()),
        },
    );
    Ok(())
}

/// premerge 闸结果。`Stopped` 表示某道闸已置态（冲突/测试失败/安全拦截）并返回。
enum GateResult {
    Pass { tested_dev_sha: Option<String> },
    Stopped,
}

/// Phase 1 dev-sync + 测试门 + 安全门。只操作 CR worktree、不碰共享 dev，故 legacy `run`
/// （持 merge_lock）与并行 `premerge_run`（仅持 cr_lock）共用。任一闸失败已置态 → `Stopped`。
async fn run_premerge_gates(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    project: &crate::models::project::Project,
    cr_id: &str,
) -> Result<GateResult> {
    // ── Phase 1：合并前自动把 dev 并入 CR 分支 ──────────────────────────────
    let dev_merged = match sync_dev_into_worktree(
        &session.worktree_path,
        &session.branch_name,
        &project.branch_dev,
    )
    .await
    {
        DevSync::Clean { dev_merged } => dev_merged,
        DevSync::Conflict { files, diff } => {
            let files_json = serde_json::to_string(&files).unwrap_or_else(|_| "[]".into());
            let report = format!(
                "## 合并冲突\n\n将 `{}` 并入 `{}` 时发生冲突（{} 个文件）：\n\n{}\n\n可在审核页三方解决、一键重试，或交由 AI 自动解冲突。",
                project.branch_dev,
                session.branch_name,
                files.len(),
                files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
            );
            let _ = sqlx::query(
                "UPDATE worktree_sessions SET conflict_files=?, conflict_diff=?, report_content=? WHERE id=?",
            )
            .bind(&files_json)
            .bind(&diff)
            .bind(&report)
            .bind(&session.id)
            .execute(db)
            .await;
            set_status(db, cr_id, &cr.issue_id, "merge_conflict").await?;
            event::emit(
                app,
                event::AppEvent::MergeConflict {
                    cr_id: cr_id.to_string(),
                    files: files.clone(),
                },
            );
            info!("pre-merge dev-sync conflict for cr {} ({} files)", cr_id, files.len());

            // 自动解冲突开关 ON → 交 AI 后台处理（不占锁）。
            if crate::core::gate::auto_conflict_resolve_enabled(db).await {
                info!("auto conflict-resolve enabled, handing cr {} to AI (background)", cr_id);
                let (db2, tx2, app2, cr2) =
                    (db.clone(), tx.clone(), app.clone(), cr_id.to_string());
                tokio::spawn(async move {
                    if let Err(e) = ai_resolve_conflict(&db2, &tx2, &app2, &cr2).await {
                        info!("AI conflict-resolve failed for {}: {}", cr2, e);
                    }
                });
            }
            return Ok(GateResult::Stopped);
        }
    };

    // 记录测试所基于的 dev 提交 SHA（land 阶段再校验用）。
    let wt = GitProxy::new(&session.worktree_path);
    let dev_ref = resolve_dev_ref(&wt, &project.branch_dev).await;
    let tested_dev_sha = wt
        .run(&["rev-parse", &dev_ref])
        .await
        .ok()
        .filter(|(c, _, _)| *c == 0)
        .map(|(_, o, _)| o.trim().to_string())
        .filter(|s| !s.is_empty());

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "testing".to_string(),
            note: Some("合并前测试门校验中…".to_string()),
        },
    );

    // Quality gate（spec: testing.md）：失败必须阻断合并。
    let passed = crate::tasks::testing::run_and_gate(db, tx, app, cr_id).await?;

    // D3：CR 级测试遥测。
    let _ = sqlx::query(
        "INSERT INTO cr_test_runs (id, cr_id, result, summary, run_by)
         VALUES (?, ?, ?, ?, 'auto')",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(cr_id)
    .bind(if passed { "pass" } else { "fail" })
    .bind(if passed { "合并前自动测试通过" } else { "合并前自动测试失败，已阻断合并" })
    .execute(db)
    .await;

    if !passed {
        // 刚把 dev 并入后失败 → merge_conflict（集成破坏）；否则 CR 自身失败 → merge_failed。
        let fail_status = if dev_merged { "merge_conflict" } else { "merge_failed" };
        let fail_msg = if dev_merged {
            "并入 dev 后测试未通过（集成破坏），已阻断合并"
        } else {
            "合并前测试未通过，已阻断合并"
        };
        info!(
            "pre-merge tests failed for cr {} (dev_merged={}), blocking merge",
            cr_id, dev_merged
        );
        // 把测试失败详情回写报告，供审核页「合并失败原因」展示有用信息（而非实现摘要）。
        crate::tasks::testing::persist_test_failure_report(db, cr_id).await;
        set_status(db, cr_id, &cr.issue_id, fail_status).await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: fail_status.to_string(),
                message: Some(fail_msg.to_string()),
            },
        );
        return Ok(GateResult::Stopped);
    }

    // Shift-left 安全门：高危发现阻断合并（复用 merge_failed 的恢复 UI）。
    if let Some(reason) =
        crate::tasks::security_audit::pre_merge_gate(db, &project.repo_path, cr_id).await
    {
        info!("pre-merge security gate blocked cr {}", cr_id);
        let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
            .bind(&reason)
            .bind(&session.id)
            .execute(db)
            .await;
        set_status(db, cr_id, &cr.issue_id, "merge_failed").await?;
        crate::core::notify::dispatch(db, "security_high", "安全门拦截合并", cr_id).await;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_failed".to_string(),
                message: Some("合并前安全扫描发现高危问题，已阻断合并".to_string()),
            },
        );
        return Ok(GateResult::Stopped);
    }

    Ok(GateResult::Pass { tested_dev_sha })
}

/// 落地 dev：dev_is_live 检测 + 合并信息 + `land_on_dev` + `finalize_merged`。
/// 调用方须已持 `merge_lock`（land 必须串行）。
async fn land_core(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    project: &crate::models::project::Project,
    cr_id: &str,
) -> Result<()> {
    let git = GitProxy::new(&project.repo_path);

    // dev 是否为主工作树当前签出分支（自管理场景）。detached HEAD 时为空 → false。
    let live_branch = git
        .run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default();
    let dev_is_live = !live_branch.is_empty() && live_branch == project.branch_dev;

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "merging".to_string(),
            note: Some(format!("正在合并到 {}…", project.branch_dev)),
        },
    );

    // 人审填写的合并信息；空则回退默认模板。
    let merge_msg = match cr
        .merge_commit_message
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => default_merge_message(db, cr).await,
    };

    let merge_commit = match land_on_dev(&git, project, session, cr_id, dev_is_live, &merge_msg).await
    {
        Ok(sha) => sha,
        Err((merge_code, merge_err)) => {
            info!("merge failed ({}): {}", merge_code, merge_err);
            let fail_reason = format!(
                "## 合并失败\n\n无法将分支 `{}` 合并到 `{}`（退出码 {}）。常见原因为代码冲突，可在修复后重新执行。\n\n```\n{}\n```\n",
                session.branch_name,
                project.branch_dev,
                merge_code,
                merge_err.chars().take(2000).collect::<String>()
            );
            let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
                .bind(&fail_reason)
                .bind(&session.id)
                .execute(db)
                .await;
            set_status(db, cr_id, &cr.issue_id, "merge_failed").await?;
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
    };

    finalize_merged(db, tx, app, &git, session, cr, merge_commit).await
}

/// 落地成功后的统一收尾：删 worktree、置 merged、终止预览、发事件、入队安全审计、Innate 后台沉淀。
async fn finalize_merged(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    git: &GitProxy,
    session: &crate::models::worktree::WorktreeSession,
    cr: &crate::models::change_request::ChangeRequest,
    merge_commit: String,
) -> Result<()> {
    let cr_id = cr.id.as_str();

    if std::path::Path::new(&session.worktree_path).exists() {
        let _ = git
            .run(&["worktree", "remove", "--force", &session.worktree_path])
            .await;
    }

    set_status(db, cr_id, &cr.issue_id, "merged").await?;

    // 持久化 squash 提交 SHA（供「撤销该需求改动」revert）；空 → NULL 优雅降级。
    let merge_commit_opt: Option<&str> = if merge_commit.is_empty() {
        None
    } else {
        Some(merge_commit.as_str())
    };
    sqlx::query(
        "UPDATE worktree_sessions SET status='merged', merge_commit=?, completed_at=datetime('now') WHERE id=?",
    )
    .bind(merge_commit_opt)
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

    // Note: tests already ran as a pre-merge gate, so no post-merge testing job here
    // (Phase 3 集成门将在合并后另行去抖触发，见实施方案 §9)。

    // Node 07：合并 diff 安全审计。
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

    // ── 合并后副作用：脱锁后台执行（通知 + Innate 蒸馏，避免占着 merge_lock）。
    {
        let db2 = db.clone();
        let cr_id2 = cr_id.to_string();
        let project_id2 = cr.project_id.clone();
        let issue_id2 = cr.issue_id.clone();
        let report = session.report_content.clone();
        tokio::spawn(async move {
            crate::core::notify::dispatch(&db2, "cr_merged", "已合并到 dev", &cr_id2).await;

            let issue_title: String =
                sqlx::query_as::<_, (String,)>("SELECT title FROM issues WHERE id=?")
                    .bind(&issue_id2)
                    .fetch_optional(&db2)
                    .await
                    .ok()
                    .flatten()
                    .map(|t| t.0)
                    .unwrap_or_default();
            if let Some(report) = report.as_deref().filter(|r| !r.trim().is_empty()) {
                let content = format!(
                    "已合并需求「{}」的成功实现方案（通过代码审核 与测试）：\n\n{}",
                    issue_title, report
                );
                let trigger =
                    format!("实现该项目同类需求时可参考的成功改动方案；相关需求：{}", issue_title);
                crate::knowledge::kb_add(&project_id2, &content, &trigger).await;
            }
            crate::knowledge::consume_recall_trace(&db2, "change_request", &cr_id2, "ok", Some("up"))
                .await;
            crate::knowledge::kb_evolve(&project_id2).await;
        });
    }

    info!("merge completed for cr {}", cr_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Build a repo on `main` with `f.txt`, then a diverged `dev` branch, and a
    /// CR worktree forked from the pre-dev `main`. Returns (repo_dir, worktree_dir).
    fn setup(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "af-merge-test-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let repo = base.join("repo");
        let wt = base.join("wt");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "tester"]);
        std::fs::write(repo.join("f.txt"), "line1\nline2\nline3\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        // dev diverges: change line2.
        git(&repo, &["branch", "dev"]);
        git(&repo, &["checkout", "-q", "dev"]);
        std::fs::write(repo.join("f.txt"), "line1\nDEV-CHANGE\nline3\n").unwrap();
        git(&repo, &["commit", "-qam", "dev edits line2"]);
        git(&repo, &["checkout", "-q", "main"]);
        // CR worktree forks from main (pre-dev state).
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "cr", wt.to_str().unwrap(), "main"],
        );
        (repo, wt)
    }

    #[tokio::test]
    async fn dev_sync_detects_conflict_and_restores_worktree() {
        let (repo, wt) = setup("conflict");
        // CR touches the SAME line2 differently → must conflict with dev.
        std::fs::write(wt.join("f.txt"), "line1\nCR-CHANGE\nline3\n").unwrap();
        git(&wt, &["commit", "-qam", "cr edits line2"]);

        let res = sync_dev_into_worktree(wt.to_str().unwrap(), "cr", "dev").await;
        match res {
            DevSync::Conflict { files, diff } => {
                assert!(files.iter().any(|f| f == "f.txt"), "expected f.txt conflict, got {files:?}");
                assert!(diff.contains("<<<<<<<"), "conflict diff should carry markers");
            }
            DevSync::Clean { .. } => panic!("expected a conflict"),
        }
        // merge --abort must have restored the tree: no leftover conflict markers.
        let content = std::fs::read_to_string(wt.join("f.txt")).unwrap();
        assert!(!content.contains("<<<<<<<"), "worktree not restored: {content}");
        assert_eq!(content, "line1\nCR-CHANGE\nline3\n");

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }

    #[tokio::test]
    async fn dev_sync_clean_when_disjoint_changes() {
        let (repo, wt) = setup("clean");
        // The fork point (main, pre-dev) — the stale base_commit a CR would carry.
        let fork = {
            let out = Command::new("git")
                .args(["rev-parse", "main"])
                .current_dir(&repo)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        // CR touches a DIFFERENT file → merges cleanly, integrating dev's edit.
        std::fs::write(wt.join("g.txt"), "new file\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qam", "cr adds g.txt"]);

        let res = sync_dev_into_worktree(wt.to_str().unwrap(), "cr", "dev").await;
        match res {
            DevSync::Clean { dev_merged } => assert!(dev_merged, "dev's line2 edit should integrate"),
            DevSync::Conflict { files, .. } => panic!("unexpected conflict on {files:?}"),
        }
        // dev's change is now present in the worktree branch.
        let content = std::fs::read_to_string(wt.join("f.txt")).unwrap();
        assert_eq!(content, "line1\nDEV-CHANGE\nline3\n");

        // Diff fix: even after the branch merged dev in — and even when a STALE
        // base_commit (the original fork) is supplied — the CR diff must show only
        // the CR's own change (g.txt) and NOT dev's f.txt edit. merge-base(dev,branch)
        // now equals dev's tip, so dev's changes are correctly excluded.
        let diff = crate::commands::change_requests::compute_worktree_diff(
            wt.to_str().unwrap(),
            "cr",
            "dev",
            Some(&fork),
        )
        .await
        .unwrap_or_default();
        assert!(diff.contains("g.txt"), "CR's own file should be in diff:\n{diff}");
        assert!(
            !diff.contains("DEV-CHANGE"),
            "dev's change must NOT leak into the CR diff:\n{diff}"
        );

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }

    fn dummy_project(repo_path: &str) -> crate::models::project::Project {
        crate::models::project::Project {
            id: "p1".into(),
            name: "p".into(),
            slug: "p".into(),
            description: String::new(),
            repo_path: repo_path.into(),
            branch_dev: "dev".into(),
            branch_main: "main".into(),
            status: "active".into(),
            config_yaml: None,
            is_default: false,
            archived_at: None,
            code_agent_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn dummy_session(worktree_path: &str, branch: &str) -> crate::models::worktree::WorktreeSession {
        crate::models::worktree::WorktreeSession {
            id: "s1".into(),
            change_request_id: "cr1".into(),
            worktree_path: worktree_path.into(),
            branch_name: branch.into(),
            status: "review".into(),
            prompt_snapshot: None,
            iteration_count: 1,
            report_content: None,
            diff_content: None,
            base_commit: None,
            merge_commit: None,
            conflict_files: None,
            conflict_diff: None,
            tested_dev_sha: None,
            premerge_at: None,
            started_at: None,
            completed_at: None,
        }
    }

    fn commit_count(dir: &Path, rev: &str) -> usize {
        let out = Command::new("git")
            .args(["rev-list", "--count", rev])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().parse().unwrap()
    }

    /// Fast-path land squashes the CR branch (multiple impl commits) into a SINGLE
    /// commit on dev carrying the human-written merge message.
    #[tokio::test]
    async fn land_on_dev_squashes_to_one_commit() {
        let (repo, wt) = setup("squash");
        // CR makes TWO commits on a DIFFERENT file (no conflict with dev's line2).
        std::fs::write(wt.join("g.txt"), "g1\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qam", "cr commit 1"]);
        std::fs::write(wt.join("g.txt"), "g1\ng2\n").unwrap();
        git(&wt, &["commit", "-qam", "cr commit 2"]);

        let before = commit_count(&repo, "dev");
        let g = GitProxy::new(repo.to_str().unwrap());
        let project = dummy_project(repo.to_str().unwrap());
        let session = dummy_session(wt.to_str().unwrap(), "cr");

        let res = land_on_dev(&g, &project, &session, "cr1", false, "merge msg X").await;
        assert!(res.is_ok(), "land should succeed: {res:?}");

        // Exactly ONE new commit on dev (squash collapses the two CR commits).
        assert_eq!(commit_count(&repo, "dev"), before + 1, "squash must add 1 commit");
        // That commit carries the human merge message, not a "Merge branch" default.
        let msg = {
            let out = Command::new("git")
                .args(["log", "-1", "--pretty=%s", "dev"])
                .current_dir(&repo)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        assert_eq!(msg, "merge msg X");
        // The CR's change is actually present on dev.
        git(&repo, &["checkout", "-q", "dev"]);
        assert_eq!(std::fs::read_to_string(repo.join("g.txt")).unwrap(), "g1\ng2\n");

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }

    /// Regression: when the CR branch is already contained in dev (its change
    /// landed via an earlier CR in the same batch), squash stages nothing. Land
    /// must SUCCEED without an empty commit — not fail like a naive squash+commit.
    #[tokio::test]
    async fn land_on_dev_noop_when_already_merged() {
        let (repo, wt) = setup("noop");
        std::fs::write(wt.join("g.txt"), "g1\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qam", "cr commit"]);
        // Land the CR's change into dev first (simulates an earlier batch sibling).
        git(&repo, &["checkout", "-q", "dev"]);
        git(&repo, &["merge", "--squash", "cr"]);
        git(&repo, &["commit", "-qam", "earlier sibling"]);
        git(&repo, &["checkout", "-q", "main"]);

        let before = commit_count(&repo, "dev");
        let g = GitProxy::new(repo.to_str().unwrap());
        let project = dummy_project(repo.to_str().unwrap());
        let session = dummy_session(wt.to_str().unwrap(), "cr");

        let res = land_on_dev(&g, &project, &session, "cr1", false, "merge msg").await;
        assert!(res.is_ok(), "no-op land should succeed, got {res:?}");
        assert_eq!(commit_count(&repo, "dev"), before, "must NOT create an empty commit");

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }

    /// Re-entrancy (startup recovery): a previous land that crashed AFTER `merge --squash`
    /// (stages changes on dev) but BEFORE `commit` leaves dev's index dirty with no
    /// MERGE_HEAD. The recovered land must `reset --hard` that residue and re-apply cleanly
    /// rather than fail on a dirty index ("local changes would be overwritten"). Asserts the
    /// recovered land succeeds with exactly one squash commit carrying the CR change.
    #[tokio::test]
    async fn land_on_dev_recovers_from_staged_squash_residue() {
        let (repo, wt) = setup("reentrant");
        std::fs::write(wt.join("g.txt"), "g1\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qam", "cr commit"]);

        // Simulate crash residue: stage the squash on dev but never commit (no MERGE_HEAD).
        git(&repo, &["checkout", "-q", "dev"]);
        git(&repo, &["merge", "--squash", "cr"]);

        let before = commit_count(&repo, "dev");
        let g = GitProxy::new(repo.to_str().unwrap());
        let project = dummy_project(repo.to_str().unwrap());
        let session = dummy_session(wt.to_str().unwrap(), "cr");

        let res = land_on_dev(&g, &project, &session, "cr1", false, "recovered merge").await;
        assert!(res.is_ok(), "recovered land should succeed despite staged residue: {res:?}");
        assert_eq!(
            commit_count(&repo, "dev"),
            before + 1,
            "exactly one squash commit after recovery (no doubling, no failure)"
        );
        git(&repo, &["checkout", "-q", "dev"]);
        assert_eq!(std::fs::read_to_string(repo.join("g.txt")).unwrap(), "g1\n");

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }

    /// revalidate 的 disjoint 判定核心①：从快照的统一 diff 解析出本 CR 改动的文件集。
    #[test]
    fn parse_changeset_files_extracts_paths() {
        let diff = "diff --git a/src/foo.rs b/src/foo.rs\n\
                    index 111..222 100644\n--- a/src/foo.rs\n+++ b/src/foo.rs\n\
                    @@ -1 +1 @@\n-old\n+new\n\
                    diff --git a/docs/README.md b/docs/README.md\n\
                    new file mode 100644\n+++ b/docs/README.md\n";
        let set = parse_changeset_files(Some(diff));
        assert!(set.contains("src/foo.rs"), "got {set:?}");
        assert!(set.contains("docs/README.md"), "got {set:?}");
        assert_eq!(set.len(), 2);
        // 无 diff / 空 → 空集（land_run 据此把「无 diff 信息」当重叠，保守补测）。
        assert!(parse_changeset_files(None).is_empty());
        assert!(parse_changeset_files(Some("")).is_empty());
    }

    /// revalidate 的 disjoint 判定核心②：`git diff --name-only a..b` 取兄弟 CR 落地的文件集。
    #[tokio::test]
    async fn diff_name_only_lists_advanced_files() {
        let (repo, _wt) = setup("nameonly");
        let sha = |rev: &str| {
            let out = Command::new("git")
                .args(["rev-parse", rev])
                .current_dir(&repo)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        // setup: dev 从 main 分叉并只改了 f.txt → diff main..dev 应恰为 {f.txt}。
        let g = GitProxy::new(repo.to_str().unwrap());
        let moved = diff_name_only(&g, &sha("main"), &sha("dev")).await;
        assert!(moved.contains("f.txt"), "got {moved:?}");
        assert_eq!(moved.len(), 1);

        // 与一个改 g.txt 的 CR 文件集 → 不相交（disjoint），land 可不重测直落。
        let cr_files: std::collections::HashSet<String> = ["g.txt".to_string()].into_iter().collect();
        assert!(moved.is_disjoint(&cr_files));
        // 与一个也改 f.txt 的 CR → 相交（overlap），land 须退回补测。
        let cr_files2: std::collections::HashSet<String> =
            ["f.txt".to_string()].into_iter().collect();
        assert!(!moved.is_disjoint(&cr_files2));

        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }
}
