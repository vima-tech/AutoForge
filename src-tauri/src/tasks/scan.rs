use crate::core::{event, security};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use crate::tasks::testing::{configured_checks, run_check};
use anyhow::{anyhow, Result};
use serde_json::json;
use uuid::Uuid;

/// Node 06 — proactive inspection (design §6.2 mode B).
/// Runs the project's full configured check suite against the main repo (not a
/// worktree), records a `proactive` test session, and files failures back as
/// issues so they re-enter the pipeline. Triggered on a daily schedule or manually.
pub async fn run_proactive(
    db: &Db,
    tx: &JobSender,
    sink: &dyn crate::core::event::EventSink,
    project_id: &str,
    trigger: &str,
) -> Result<()> {
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(project_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("project {} not found", project_id))?;

    // No configured check commands → there is nothing meaningful to inspect.
    // Bail out *before* creating a session so we don't litter the history with
    // empty "巡检跳过" records; callers surface this as a prompt instead.
    let checks = configured_checks(crate::commands::run_config::effective_config(&project).as_deref());
    if checks.is_empty() {
        return Err(anyhow!(
            "项目「{}」未配置任何检查命令，无需巡检。请先在「项目管理 → 配置」中填写 test/quality 检查命令。",
            project.name
        ));
    }

    let session_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO test_sessions
         (id, project_id, session_type, trigger, status, started_at)
         VALUES (?, ?, 'proactive', ?, 'running', datetime('now'))",
    )
    .bind(&session_id)
    .bind(&project.id)
    .bind(trigger)
    .execute(db)
    .await?;

    let mut results = Vec::new();
    for (name, command, timeout) in checks {
        results.push(run_check(&project.repo_path, name, command, timeout).await);
    }

    let failed: Vec<_> = results.iter().filter(|r| !r.ok).collect();
    let status = if failed.is_empty() { "passed" } else { "failed" };
    let summary = if failed.is_empty() {
        format!("巡检通过：{} 项检查全部正常", results.len())
    } else {
        format!("巡检发现 {} / {} 项检查失败", failed.len(), results.len())
    };

    let results_json = json!({
        "checks": results.iter().map(|r| json!({
            "name": r.name,
            "command": r.command,
            "ok": r.ok,
            "code": r.code,
            "stdout": r.stdout.chars().take(4000).collect::<String>(),
            "stderr": r.stderr.chars().take(4000).collect::<String>(),
        })).collect::<Vec<_>>()
    })
    .to_string();

    // File each failing check as a quality issue (deduped by fingerprint).
    let mut issues_created: Vec<String> = Vec::new();
    for r in &failed {
        let title = format!("巡检发现问题：{} ({})", r.name, project.name);
        let description = format!(
            "主动巡检检查 `{}` 失败。\n\n命令：{}\n退出码：{}\n\nstderr:\n{}",
            r.name, r.command, r.code, r.stderr
        );
        let fp = security::fingerprint(&title, &description);
        let issue_id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO issues
             (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'scan', ?, ?, 'Bug', 'medium', 6, 'pending_analysis', ?)",
        )
        .bind(&issue_id)
        .bind(&project.id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .execute(db)
        .await?;
        if inserted.rows_affected() > 0 {
            issues_created.push(issue_id.clone());
            let _ = enqueue(
                db,
                tx,
                "analysis",
                &format!("analysis:{}", issue_id),
                JobPayload::Analysis { issue_id },
            )
            .await;
        }
    }

    sqlx::query(
        "UPDATE test_sessions
         SET status=?, summary=?, results_json=?, issues_created=?, completed_at=datetime('now')
         WHERE id=?",
    )
    .bind(status)
    .bind(&summary)
    .bind(&results_json)
    .bind(serde_json::to_string(&issues_created)?)
    .bind(&session_id)
    .execute(db)
    .await?;

    event::emit(
        sink,
        event::AppEvent::TestCompleted {
            cr_id: String::new(),
            test_session_id: session_id,
            status: status.to_string(),
            summary,
        },
    );

    Ok(())
}
