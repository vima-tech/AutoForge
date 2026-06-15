//! Innate integration — AutoForge self-growth knowledge layer.
//!
//! Strategy: shell out to the `innate` CLI (the same integration path Innate's
//! own SDKs/Daemon use) rather than linking it in-process, which avoids the
//! rusqlite vs sqlx/libsqlite3-sys SQLite symbol clash. All operations are
//! best-effort: if the `innate` binary is absent or errors, the pipeline runs
//! unchanged. Tenancy = per-project db + a shared db (design: shared = the
//! factory's durable craft; proj = disposable working memory).

use crate::state::kb_base;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

fn innate_bin() -> String {
    std::env::var("AUTOFORGE_INNATE_BIN").unwrap_or_else(|_| "innate".to_string())
}

fn proj_db(project_id: &str) -> PathBuf {
    PathBuf::from(kb_base()).join(format!("proj-{}.db", sanitize(project_id)))
}

fn shared_db() -> PathBuf {
    PathBuf::from(kb_base()).join("shared.db")
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

async fn run_innate(db: &PathBuf, args: &[&str], timeout_secs: u64) -> Option<String> {
    let _ = tokio::fs::create_dir_all(kb_base()).await;
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        Command::new(innate_bin())
            .arg("--db")
            .arg(db)
            .args(args)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if result.status.success() {
        Some(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        None
    }
}

/// Recall procedural knowledge for a coding/analysis prompt.
/// Fans out over the project db ⊕ shared db (project-first, shared as backstop)
/// and returns injectable text. Empty string when nothing is available.
pub async fn kb_recall(project_id: &str, query: &str) -> String {
    let q: String = query.chars().take(500).collect();
    if q.trim().is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    if let Some(text) = run_innate(
        &proj_db(project_id),
        &["recall", &q, "--budget", "3000"],
        45,
    )
    .await
    {
        if !text.trim().is_empty() {
            parts.push(format!("### 项目经验\n{}", text.trim()));
        }
    }
    if let Some(text) = run_innate(&shared_db(), &["recall", &q, "--budget", "2000"], 45).await {
        if !text.trim().is_empty() {
            parts.push(format!("### 通用技能（跨项目）\n{}", text.trim()));
        }
    }
    parts.join("\n\n")
}

/// Capture a knowledge chunk into the project db (best-effort, fire-and-forget safe).
/// `trigger` is the "when to apply" description that drives Innate's trigger vector.
pub async fn kb_add(project_id: &str, content: &str, trigger: &str) {
    let content: String = content.chars().take(4000).collect();
    let trigger: String = trigger.chars().take(300).collect();
    let _ = run_innate(
        &proj_db(project_id),
        &["add", &content, "--kind", "note", "--trigger", &trigger],
        45,
    )
    .await;
}

/// Distil + curate a project's accumulated episodic logs into knowledge.
pub async fn kb_evolve(project_id: &str) {
    let _ = run_innate(&proj_db(project_id), &["evolve", "--trigger", "manual"], 120).await;
}
