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

/// 方案 B：把合并冲突交给 code agent 自动消解，复跑测试后【回到审核 2】复审，
/// 绝不直接落 dev。手动「AI 解冲突并合并」按钮与 Phase 1 自动开关共用此函数。
pub async fn ai_resolve_conflict(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    cr_id: &str,
) -> Result<()> {
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
    let _ = wt.run(&["config", "rerere.enabled", "true"]).await;
    let _ = wt.run(&["fetch", "origin", &project.branch_dev]).await;
    let remote_ref = format!("origin/{}", project.branch_dev);
    let dev_ref = if wt
        .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false)
    {
        remote_ref
    } else {
        project.branch_dev.clone()
    };
    // Re-create the conflict in the tree (Phase 1 aborted it). rerere may auto-resolve.
    let merge_msg = format!("AutoForge: sync {} into {}", dev_ref, session.branch_name);
    let (mc, _, _) = wt
        .run(&["merge", "-m", &merge_msg, &dev_ref])
        .await
        .unwrap_or((-1, String::new(), String::new()));

    if mc != 0 {
        let unmerged: Vec<String> = wt
            .run(&["diff", "--name-only", "--diff-filter=U"])
            .await
            .ok()
            .map(|(_, o, _)| {
                o.lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default();
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
        let (_code, report, _err) =
            crate::agents::code_agent::run(&session.worktree_path, &prompt, 600)
                .await
                .unwrap_or((-1, String::new(), String::new()));
        // agent 输出视为外部输入：留档/回灌前过注入检测（命中只记录，文件改动才是结果）。
        if crate::core::security::has_obvious_injection(&report) {
            info!(
                "AI conflict-resolve report for {} tripped injection filter",
                cr_id
            );
        }
    }

    // Stage and verify the conflict markers are fully gone.
    let _ = wt.run(&["add", "-A"]).await;
    let markers_clean = wt
        .run(&["diff", "--cached", "--check"])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);
    let still_unmerged = wt
        .run(&["diff", "--name-only", "--diff-filter=U"])
        .await
        .ok()
        .map(|(_, o, _)| !o.trim().is_empty())
        .unwrap_or(false);

    if !markers_clean || still_unmerged {
        let _ = wt.run(&["merge", "--abort"]).await;
        sqlx::query("UPDATE change_requests SET status='merge_conflict', updated_at=datetime('now') WHERE id=?")
            .bind(cr_id).execute(db).await?;
        sqlx::query("UPDATE issues SET status='merge_conflict', updated_at=datetime('now') WHERE id=?")
            .bind(&cr.issue_id).execute(db).await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_conflict".to_string(),
                message: Some("AI 未能完全解决冲突，仍需人工处理".to_string()),
            },
        );
        return Ok(());
    }

    let commit_msg = format!("AutoForge: AI 解决合并冲突（{} → {}）", dev_ref, session.branch_name);
    let _ = wt.run(&["commit", "-m", &commit_msg]).await;
    let _ = sqlx::query(
        "UPDATE worktree_sessions SET conflict_files=NULL, conflict_diff=NULL WHERE id=?",
    )
    .bind(&session.id)
    .execute(db)
    .await;

    // Re-run the test gate on the integrated result. Failure blocks (merge_failed).
    let passed = crate::tasks::testing::run_and_gate(db, tx, app, cr_id)
        .await
        .unwrap_or(false);
    if !passed {
        sqlx::query("UPDATE change_requests SET status='merge_failed', updated_at=datetime('now') WHERE id=?")
            .bind(cr_id).execute(db).await?;
        sqlx::query("UPDATE issues SET status='merge_failed', updated_at=datetime('now') WHERE id=?")
            .bind(&cr.issue_id).execute(db).await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_failed".to_string(),
                message: Some("AI 解冲突后测试未通过，已阻断合并".to_string()),
            },
        );
        return Ok(());
    }

    // Route back to human review 2 — never land an AI-resolved conflict directly.
    sqlx::query("UPDATE change_requests SET status='pending_review_2', updated_at=datetime('now') WHERE id=?")
        .bind(cr_id).execute(db).await?;
    sqlx::query("UPDATE issues SET status='pending_review_2', updated_at=datetime('now') WHERE id=?")
        .bind(&cr.issue_id).execute(db).await?;
    crate::core::notify::dispatch(db, "review_needed", &issue.title, "AI 已解决合并冲突，待审核 2 复审").await;
    event::emit(
        app,
        event::AppEvent::ReviewNeeded {
            cr_id: cr_id.to_string(),
            issue_title: issue.title,
            stage: 2,
        },
    );
    info!("AI resolved merge conflict for cr {}, routed back to review 2", cr_id);
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

    // ── Phase 0：同项目合并串行 ─────────────────────────────────────────────
    // 全程持有该项目的合并锁，避免并发 `checkout dev && merge` 互相踩同一个 dev
    // 工作树（竞态）；跨项目仍并行。锁在函数返回时随 guard 释放。
    let merge_lock = crate::state::merge_lock(&cr.project_id);
    let _merge_guard = merge_lock.lock().await;

    // 在把 dev 并入分支【之前】快照本 CR 相对分叉点的 diff——这样持久化的 diff 始终
    // 只含本 CR 的改动（merge dev 进来后再按 base_commit 做 diff 会把 dev 的无关改动
    // 也算进来，重新污染）。worktree 删除后审核页据此回看已合并需求的改动。
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

    // ── Phase 1：合并前自动把 dev 并入 CR 分支 ──────────────────────────────
    // 让下面的测试门跑在【集成后】的代码上，且最终 land_on_dev 必为无冲突合并。
    // 冲突 → 保留现场、置 merge_conflict，按自动开关决定是否走 AI 自动解冲突。
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
            sqlx::query("UPDATE change_requests SET status='merge_conflict', updated_at=datetime('now') WHERE id=?")
                .bind(cr_id).execute(db).await?;
            sqlx::query("UPDATE issues SET status='merge_conflict', updated_at=datetime('now') WHERE id=?")
                .bind(&cr.issue_id).execute(db).await?;
            event::emit(
                app,
                event::AppEvent::MergeConflict {
                    cr_id: cr_id.to_string(),
                    files: files.clone(),
                },
            );
            info!("pre-merge dev-sync conflict for cr {} ({} files)", cr_id, files.len());

            // 自动解冲突开关 ON → 交 AI 处理。**spawn 到后台**而非在持有合并锁时 await：
            // AI 解冲突可能跑数分钟（claude CLI），期间不应占着该项目的合并锁饿死其它 CR。
            // ai_resolve 只在本 CR 自己的 worktree 内操作、解完回审核 2（绝不落 dev），与其它
            // 合并无共享可变状态，脱锁后台执行安全。失败则维持 merge_conflict 等人。
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
            return Ok(());
        }
    };

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "testing".to_string(),
            note: Some("合并前测试门校验中…".to_string()),
        },
    );

    // Quality gate: run configured tests on the un-merged worktree branch
    // FIRST. A failing gate must block the merge (spec: testing.md).
    let passed = crate::tasks::testing::run_and_gate(db, tx, app, cr_id).await?;

    // D3：CR 级测试遥测——把这次合并前自动测试的整体结果落一条记录（吞吐质量趋势）。
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
        // 若失败发生在【刚把 dev 并入分支】之后，归为 merge_conflict（集成破坏，走冲突
        // 兜底 UI / AI 解冲突）；否则是 CR 自身测试失败 → 维持 merge_failed。均阻断落地。
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
        sqlx::query("UPDATE change_requests SET status=?, updated_at=datetime('now') WHERE id=?")
            .bind(fail_status)
            .bind(cr_id)
            .execute(db)
            .await?;
        sqlx::query("UPDATE issues SET status=?, updated_at=datetime('now') WHERE id=?")
            .bind(fail_status)
            .bind(&cr.issue_id)
            .execute(db)
            .await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: fail_status.to_string(),
                message: Some(fail_msg.to_string()),
            },
        );
        return Ok(());
    }

    // Shift-left security gate: a fast, deterministic heuristic scan of the CR
    // diff runs BEFORE the merge. High/critical findings (secrets, rm -rf /,
    // os.system, shell=True, …) block the merge so risky code never reaches dev.
    // Reuses `merge_failed` so the existing audit recovery UI (retry/delete)
    // surfaces it; the report makes clear it is a security block, not a conflict.
    if let Some(reason) =
        crate::tasks::security_audit::pre_merge_gate(db, &project.repo_path, cr_id).await
    {
        info!("pre-merge security gate blocked cr {}", cr_id);
        let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
            .bind(&reason)
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
        crate::core::notify::dispatch(db, "security_high", "安全门拦截合并", cr_id).await;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "merge_failed".to_string(),
                message: Some("合并前安全扫描发现高危问题，已阻断合并".to_string()),
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

    // (CR diff was snapshotted earlier — before Phase 1 merged dev into the branch —
    // so it stays scoped to the CR's own changes against its fork point.)

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "merging".to_string(),
            note: Some(format!("正在合并到 {}…", project.branch_dev)),
        },
    );

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
    crate::knowledge::consume_recall_trace(db, "change_request", cr_id, "ok", Some("up")).await;

    // Innate: distil this project's accumulated logs into knowledge after a successful merge.
    crate::knowledge::kb_evolve(&cr.project_id).await;

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
}
