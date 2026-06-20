use crate::core::{event, git::GitProxy};
use crate::db::Db;
use crate::state::worktrees_base;
use anyhow::{anyhow, Result};
use tracing::info;
use uuid::Uuid;

/// Compose a human-readable failure reason (exit code + trailing output) so the
/// audit page can surface *why* a run failed without anyone opening the logs.
fn build_failure_reason(exit_code: i32, stdout: &str, stderr: &str) -> String {
    fn tail(s: &str, max: usize) -> String {
        let s = s.trim();
        if s.chars().count() <= max {
            s.to_string()
        } else {
            let start = s.chars().count() - max;
            format!("…{}", s.chars().skip(start).collect::<String>())
        }
    }
    let mut out = format!("## 执行失败\n\n退出码：{}\n", exit_code);
    let so = tail(stdout, 2000);
    if !so.is_empty() {
        out.push_str(&format!("\n### 输出\n```\n{}\n```\n", so));
    }
    let se = tail(stderr, 1000);
    if !se.is_empty() {
        out.push_str(&format!("\n### 错误\n```\n{}\n```\n", se));
    }
    out
}

pub async fn run(
    db: &Db,
    tx: &crate::tasks::runner::JobSender,
    app: &tauri::AppHandle,
    cr_id: &str,
    _project_id: &str,
) -> Result<()> {
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

    let analysis_summary = analysis
        .as_ref()
        .map(|a| a.analysis_summary.clone())
        .unwrap_or_default();
    // Full structured spec (issue_analysis.schema.json) drives the Claude Code work order.
    let analysis_spec = analysis
        .as_ref()
        .and_then(|a| a.analysis_json.as_deref())
        .and_then(crate::agents::analysis::parse_spec);

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

    // Ensure the base (dev) branch exists. Many repos only have main/master, so
    // create the dev integration branch from main (or HEAD) when it's missing —
    // otherwise `worktree add ... dev` fails with "invalid reference: dev".
    let dev_exists = git
        .run(&["rev-parse", "--verify", "--quiet", &project.branch_dev])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);
    if !dev_exists {
        let main_exists = git
            .run(&["rev-parse", "--verify", "--quiet", &project.branch_main])
            .await
            .map(|(c, _, _)| c == 0)
            .unwrap_or(false);
        let base = if main_exists {
            project.branch_main.as_str()
        } else {
            "HEAD"
        };
        let (bc, _, be) = git
            .run(&["branch", &project.branch_dev, base])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if bc == 0 {
            info!(
                "created base branch '{}' from '{}' for {}",
                project.branch_dev, base, cr_id
            );
        } else {
            info!(
                "could not create base branch '{}' from '{}': {}",
                project.branch_dev, base, be
            );
        }
    }

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
        // Persist a failed session so the audit page can show the git error
        // (this path runs before the normal session record is created).
        let fail_reason = build_failure_reason(wt_code, "", &wt_err);
        let _ = sqlx::query(
            "INSERT INTO worktree_sessions
             (id, change_request_id, worktree_path, branch_name, status, prompt_snapshot, iteration_count, report_content, started_at, completed_at)
             VALUES (?, ?, ?, ?, 'failed', '', ?, ?, datetime('now'), datetime('now'))",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(cr_id)
        .bind(&worktree_path)
        .bind(&branch_name)
        .bind(iteration)
        .bind(&fail_reason)
        .execute(db)
        .await;
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

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "worktree_ready".to_string(),
            note: Some(format!("已创建工作区分支 {}（第 {} 轮）", branch_name, iteration)),
        },
    );

    // Innate: recall procedural knowledge (project ⊕ shared) into the coding prompt.
    // Traced recall captures the project-db trace id + chunk ids so the eventual
    // outcome (merge = success / review_2 reject = failure) closes the loop.
    let recall = crate::knowledge::kb_recall_traced(
        &cr.project_id,
        &format!("{} {}", issue.title, issue.description),
    )
    .await;
    crate::knowledge::store_recall_trace(db, "change_request", cr_id, &cr.project_id, &recall).await;
    let analysis_summary = if recall.text.trim().is_empty() {
        analysis_summary
    } else {
        format!(
            "{}\n\n## 历史经验与技能（Innate 召回）\n{}",
            analysis_summary, recall.text
        )
    };

    // Record the fork point: the worktree's HEAD right after creation == dev's tip
    // before the agent commits anything. This is the immutable base the CR diff is
    // scoped against, so a later independent advance of dev can't pollute the diff.
    let base_commit = GitProxy::new(&worktree_path)
        .run(&["rev-parse", "HEAD"])
        .await
        .ok()
        .filter(|(code, _, _)| *code == 0)
        .map(|(_, out, _)| out.trim().to_string())
        .filter(|s| !s.is_empty());

    // Create WorktreeSession record
    let session_id = Uuid::new_v4().to_string();
    let prompt = crate::agents::code_agent::build_prompt(
        &issue.title,
        &issue.description,
        &analysis_summary,
        analysis_spec.as_ref(),
        cr.admin_suggestions_2
            .as_deref()
            .or(cr.admin_suggestions_1.as_deref()),
        iteration as u32,
        &project.repo_path,
        crate::commands::run_config::effective_config(&project).as_deref(),
    );

    sqlx::query(
        "INSERT INTO worktree_sessions
         (id, change_request_id, worktree_path, branch_name, status, prompt_snapshot, iteration_count, base_commit, started_at)
         VALUES (?, ?, ?, ?, 'running', ?, ?, ?, datetime('now'))"
    )
    .bind(&session_id)
    .bind(cr_id)
    .bind(&worktree_path)
    .bind(&branch_name)
    .bind(&prompt)
    .bind(iteration)
    .bind(&base_commit)
    .execute(db)
    .await?;

    let preview_id = Uuid::new_v4().to_string();
    // No live URL yet — the per-CR worktree preview server (commands/cr_preview.rs)
    // fills preview_url on demand. A `file://` path here is not loadable in an iframe
    // and only misled the审核 page; leave it empty until a server is actually started.
    let preview_url = String::new();
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
            preview_url: None,
        },
    );

    // M5: best-effort field masking on the preview database (design §5.3).
    {
        let dbc = db.clone();
        let proj = project.clone();
        let pid = preview_id.clone();
        let wt = worktree_path.clone();
        tokio::spawn(async move {
            crate::tasks::preview::apply_preview_masking(&dbc, &proj, &pid, &wt).await;
        });
    }

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

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "coding".to_string(),
            note: Some("调用 Claude Code 实现中（最长 30 分钟）…".to_string()),
        },
    );

    // Run the configured code agent (claude / codex / opencode), resolved per
    // project → global default → claude fallback.
    let timeout_secs = 1800; // 30 minutes
    let code_agent = crate::agents::code_agent::resolve(db, &project).await;
    info!("cr {} 使用代码 Agent: {}", cr_id, code_agent.kind());
    let (exit_code, stdout, stderr) = code_agent
        .run(&worktree_path, &prompt, timeout_secs)
        .await
        .unwrap_or_else(|e| (-1, format!("Agent error: {}", e), String::new()));

    let report = crate::agents::code_agent::extract_report(&stdout).to_string();

    // Update session
    let new_status = if exit_code == 0 {
        "completed"
    } else {
        "failed"
    };
    // On failure, store a readable reason (exit code + output) instead of a raw
    // dump so it renders cleanly in the audit page's failure panel.
    let report_to_store = if exit_code == 0 {
        report.clone()
    } else {
        build_failure_reason(exit_code, &stdout, &stderr)
    };
    sqlx::query(
        "UPDATE worktree_sessions SET status=?, report_content=?, completed_at=datetime('now') WHERE id=?"
    )
    .bind(new_status)
    .bind(&report_to_store)
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
                message: Some(report_to_store.chars().take(200).collect()),
            },
        );
        return Err(anyhow!("code agent failed for {}", cr_id));
    }

    // Claude Code runs with `Bash(git *)` disallowed, so it edits files in the
    // worktree but never commits. We commit its work onto the CR branch here;
    // without this the branch stays identical to dev, leaving the review diff
    // empty (the audit page would hang on "加载中…") and turning the eventual
    // `git merge --no-ff` into a no-op.
    let wt_git = GitProxy::new(&worktree_path);
    let has_changes = wt_git
        .run(&["status", "--porcelain"])
        .await
        .map(|(_, out, _)| !out.trim().is_empty())
        .unwrap_or(false);
    if !has_changes {
        // The agent ran cleanly (exit 0) but produced no diff. This is a
        // legitimate terminal outcome — typically the agent concluded the
        // requirement was a misjudgment / already satisfied / a no-op — not a
        // failure. Surface it as `no_change_needed` so it doesn't masquerade as
        // a red error, and preserve the agent's own explanation (its report)
        // verbatim so the operator can see *why* nothing changed.
        let note = "ℹ️ Agent 执行完成，但分析后认为无需改动代码（未产生 diff）。以下为 Agent 的说明：";
        let report_body = if report.trim().is_empty() {
            "（Agent 未给出说明）".to_string()
        } else {
            report.clone()
        };
        let no_change_report = format!("{}\n\n{}", note, report_body);
        sqlx::query(
            "UPDATE worktree_sessions SET status='no_change', report_content=?, completed_at=datetime('now') WHERE id=?",
        )
        .bind(&no_change_report)
        .bind(&session_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE change_requests SET status='no_change_needed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(cr_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE issues SET status='no_change_needed', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&issue.id)
        .execute(db)
        .await?;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "no_change_needed".to_string(),
                message: Some("Agent 分析后认为无需改动代码".to_string()),
            },
        );
        return Ok(());
    }
    let _ = wt_git.run(&["add", "-A"]).await;
    let commit_msg = format!("AutoForge: {} (i{})", issue.title, iteration);
    let (commit_code, _, commit_err) = wt_git
        .run(&[
            "-c",
            "user.name=AutoForge",
            "-c",
            "user.email=autoforge@local",
            "commit",
            "-m",
            &commit_msg,
        ])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if commit_code != 0 {
        let reason = build_failure_reason(
            commit_code,
            "",
            &format!("提交 worktree 改动失败：{}", commit_err),
        );
        sqlx::query(
            "UPDATE worktree_sessions SET status='failed', report_content=?, completed_at=datetime('now') WHERE id=?",
        )
        .bind(&reason)
        .bind(&session_id)
        .execute(db)
        .await?;
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
                message: Some(reason.chars().take(200).collect()),
            },
        );
        // Innate: close the code recall trace on terminal failure so it doesn't
        // leak. Outcome is "fail" but feedback is None — a commit failure (often
        // "no changes produced") is too ambiguous to demote the recalled chunks;
        // we record the negative outcome for evolution without yanking confidence.
        crate::knowledge::consume_recall_trace(db, "change_request", cr_id, "fail", None).await;
        return Err(anyhow!("commit failed for {}: {}", cr_id, commit_err));
    }

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "grading".to_string(),
            note: Some("已提交改动，正在进行风险评级与安全预检…".to_string()),
        },
    );

    // §7: grade the diff for risk tier.
    let grade =
        crate::agents::grader::grade(db, &project, &worktree_path, None, iteration).await;
    let _ = sqlx::query(
        "INSERT OR REPLACE INTO cr_grades (change_request_id, tier, score, rationale, change_class)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(cr_id)
    .bind(&grade.tier)
    .bind(grade.score)
    .bind(&grade.rationale)
    .bind(&grade.change_class)
    .execute(db)
    .await;

    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "completed".to_string(),
            message: Some(report.chars().take(200).collect()),
        },
    );

    // §7 gate downgrade: low-risk (T0/T1) changes whose class has earned 'auto'
    // trust skip human review 2 and merge directly. Hard-floor (T3) and untrusted
    // classes still require review 2. Globally OFF by default.
    if crate::core::gate::should_auto_pass(db, &grade.tier, &grade.change_class).await {
        sqlx::query(
            "UPDATE change_requests SET status='pending_merge', updated_at=datetime('now') WHERE id=?",
        )
        .bind(cr_id)
        .execute(db)
        .await?;
        sqlx::query(
            "UPDATE issues SET status='pending_merge', updated_at=datetime('now') WHERE id=?",
        )
        .bind(&issue.id)
        .execute(db)
        .await?;
        let _ = sqlx::query(
            "INSERT INTO admin_decisions (id, project_id, issue_id, change_request_id, stage, decision, admin_id, suggestions)
             VALUES (?, ?, ?, ?, 'review_2', 'auto_approved', 'auto', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&cr.project_id)
        .bind(&issue.id)
        .bind(cr_id)
        .bind(format!("门控降级自动放行 [{} · {}]", grade.tier, grade.change_class))
        .execute(db)
        .await;
        let _ = crate::tasks::runner::enqueue(
            db,
            tx,
            "merge",
            &format!("merge:{}", cr_id),
            crate::models::job::JobPayload::Merge { change_request_id: cr_id.to_string() },
        )
        .await;
        crate::core::notify::dispatch(db, "auto_merged", &issue.title, "低风险改动已自动放行合并").await;
        event::emit(
            app,
            event::AppEvent::WorktreeUpdate {
                cr_id: cr_id.to_string(),
                status: "auto_merged".to_string(),
                message: Some(format!("门控降级自动放行 [{}]", grade.tier)),
            },
        );
        info!("execution auto-passed (gate downgrade) for cr {}", cr_id);
        return Ok(());
    }

    // Standard path: stop for human review 2.
    sqlx::query(
        "UPDATE change_requests SET status='pending_code_review', updated_at=datetime('now') WHERE id=?",
    )
    .bind(cr_id)
    .execute(db)
    .await?;
    sqlx::query(
        "UPDATE issues SET status='pending_code_review', updated_at=datetime('now') WHERE id=?",
    )
    .bind(&issue.id)
    .execute(db)
    .await?;

    crate::core::notify::dispatch(db, "review_needed", &issue.title, "代码实现完成，待代码审核").await;

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
