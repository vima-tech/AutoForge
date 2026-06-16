use crate::core::{event, security};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    test: Option<TestConfig>,
    quality: Option<QualityConfig>,
}

#[derive(Debug, Deserialize)]
struct TestConfig {
    unit: Option<CommandConfig>,
    integration: Option<CommandConfig>,
}

#[derive(Debug, Deserialize)]
struct CommandConfig {
    command: String,
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct QualityConfig {
    lint: Option<String>,
    typing: Option<String>,
    security: Option<String>,
}

pub(crate) struct CheckResult {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) ok: bool,
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Job entry point (kept for the dispatch table). Delegates to `run_and_gate`
/// and discards the pass/fail result.
pub async fn run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    run_and_gate(db, tx, app, cr_id).await.map(|_| ())
}

/// Run the project's configured checks for a change request.
///
/// Checks run against the CR's worktree (the not-yet-merged branch) when one is
/// still on disk, otherwise against the project repo. Returns `Ok(true)` when
/// every check passes (or none are configured) and `Ok(false)` when any fails.
/// On failure a follow-up bug issue and scan finding are recorded so the failure
/// is tracked even though the merge is blocked upstream.
pub async fn run_and_gate(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    cr_id: &str,
) -> Result<bool> {
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

    // Prefer the CR's worktree (the un-merged branch) so tests gate BEFORE the
    // merge instead of validating dev after the fact.
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?;
    let test_path = session
        .as_ref()
        .map(|s| s.worktree_path.clone())
        .filter(|p| std::path::Path::new(p).exists())
        .unwrap_or_else(|| project.repo_path.clone());

    let session_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO test_sessions
         (id, project_id, session_type, change_request_id, trigger, status, started_at)
         VALUES (?, ?, 'reactive', ?, 'pre_merge', 'running', datetime('now'))",
    )
    .bind(&session_id)
    .bind(&project.id)
    .bind(cr_id)
    .execute(db)
    .await?;

    let checks = configured_checks(crate::commands::run_config::effective_config(&project).as_deref());
    let mut results = Vec::new();
    for (name, command, timeout) in checks {
        results.push(run_check(&test_path, name, command, timeout).await);
    }

    let failed = results.iter().filter(|r| !r.ok).collect::<Vec<_>>();
    let status = if failed.is_empty() {
        "passed"
    } else {
        "failed"
    };
    let summary = if results.is_empty() {
        "未配置测试命令，标记为通过".to_string()
    } else if failed.is_empty() {
        format!("{} 项检查全部通过", results.len())
    } else {
        format!("{} / {} 项检查失败", failed.len(), results.len())
    };

    let results_json = serde_json::json!({
        "checks": results.iter().map(|r| serde_json::json!({
            "name": r.name,
            "command": r.command,
            "ok": r.ok,
            "code": r.code,
            "stdout": r.stdout.chars().take(4000).collect::<String>(),
            "stderr": r.stderr.chars().take(4000).collect::<String>(),
        })).collect::<Vec<_>>()
    })
    .to_string();

    let mut issues_created = Vec::new();
    if !failed.is_empty() {
        let title = format!("合并前测试失败：{}", cr_id);
        let description = failed
            .iter()
            .map(|r| {
                format!(
                    "检查 `{}` 失败。\n\n命令：{}\n退出码：{}\n\nstderr:\n{}",
                    r.name, r.command, r.code, r.stderr
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        let issue_id = Uuid::new_v4().to_string();
        let fp = security::fingerprint(&title, &description);
        sqlx::query(
            "INSERT INTO issues
             (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'scan', ?, ?, 'Bug', 'high', 8, 'pending_analysis', ?)",
        )
        .bind(&issue_id)
        .bind(&project.id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .execute(db)
        .await?;

        let finding_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO scan_findings
             (id, test_session_id, check_type, severity, title, description, fingerprint, issue_entry_id)
             VALUES (?, ?, 'test', 'high', ?, ?, ?, ?)",
        )
        .bind(&finding_id)
        .bind(&session_id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .bind(&issue_id)
        .execute(db)
        .await?;

        issues_created.push(issue_id.clone());
        let _ = enqueue(
            db,
            tx,
            "analysis",
            &format!("analysis:{}", issue_id),
            JobPayload::Analysis {
                issue_id: issue_id.clone(),
            },
        )
        .await;
        event::emit(
            app,
            event::AppEvent::IssueCreated {
                issue_id,
                project_id: project.id.clone(),
            },
        );
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

    if status == "failed" {
        crate::core::notify::dispatch(db, "test_failed", "测试失败", &summary).await;
    }

    event::emit(
        app,
        event::AppEvent::TestCompleted {
            cr_id: cr_id.to_string(),
            test_session_id: session_id,
            status: status.to_string(),
            summary,
        },
    );

    Ok(failed.is_empty())
}

pub(crate) fn configured_checks(config_yaml: Option<&str>) -> Vec<(String, String, u64)> {
    let Some(raw) = config_yaml else {
        return Vec::new();
    };
    let Ok(config) = serde_yaml::from_str::<ProjectConfig>(raw) else {
        return Vec::new();
    };

    let mut checks = Vec::new();
    if let Some(test) = config.test {
        if let Some(unit) = test.unit {
            checks.push((
                "unit".to_string(),
                unit.command,
                unit.timeout.unwrap_or(120),
            ));
        }
        if let Some(integration) = test.integration {
            checks.push((
                "integration".to_string(),
                integration.command,
                integration.timeout.unwrap_or(300),
            ));
        }
    }
    if let Some(quality) = config.quality {
        if let Some(command) = quality.lint {
            checks.push(("lint".to_string(), command, 120));
        }
        if let Some(command) = quality.typing {
            checks.push(("typing".to_string(), command, 120));
        }
        if let Some(command) = quality.security {
            checks.push(("security".to_string(), command, 120));
        }
    }
    checks
}

pub(crate) async fn run_check(
    repo_path: &str,
    name: String,
    command: String,
    timeout_secs: u64,
) -> CheckResult {
    let mut cmd = Command::new("sh");
    cmd.arg("-lc")
        .arg(&command)
        .current_dir(repo_path)
        // Own process group so a timeout can be reaped, and ensure the child is
        // killed if this future is dropped (e.g. on timeout) instead of leaking.
        .process_group(0)
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output()).await;

    match output {
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            CheckResult {
                name,
                command,
                ok: output.status.success(),
                code,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            }
        }
        Ok(Err(e)) => CheckResult {
            name,
            command,
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: e.to_string(),
        },
        Err(_) => CheckResult {
            name,
            command,
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("timeout after {}s", timeout_secs),
        },
    }
}
