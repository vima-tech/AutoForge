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
        // Abort the half-applied merge so the dev branch / worktree stays clean
        // for a later retry, then surface the reason in the audit page.
        let _ = git.run(&["merge", "--abort"]).await;
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

    // Node 07: run a security audit on the merged diff in parallel with testing.
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

    // Innate: distil this project's accumulated logs into knowledge after a successful merge.
    crate::knowledge::kb_evolve(&cr.project_id).await;

    info!("merge completed for cr {}", cr_id);
    Ok(())
}
