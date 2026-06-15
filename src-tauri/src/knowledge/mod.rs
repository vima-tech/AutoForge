//! Innate integration — AutoForge self-growth knowledge layer.
//!
//! Strategy: shell out to the `innate` CLI (the same integration path Innate's
//! own SDKs/Daemon use) rather than linking it in-process, which avoids the
//! rusqlite vs sqlx/libsqlite3-sys SQLite symbol clash. All operations are
//! best-effort: if the `innate` binary is absent or errors, the pipeline runs
//! unchanged. Tenancy = per-project db + a shared db (design: shared = the
//! factory's durable craft; proj = disposable working memory).

use crate::state::kb_base;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
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

/// Resolve the db a scope maps to: `Some(project)` → that project's working
/// memory; `None` (e.g. a direct chat with no bound project) → the shared
/// cross-project craft db.
fn scope_db(project_id: Option<&str>) -> PathBuf {
    match project_id {
        Some(pid) if !pid.trim().is_empty() => proj_db(pid),
        _ => shared_db(),
    }
}

fn scope_key(project_id: Option<&str>) -> String {
    match project_id {
        Some(pid) if !pid.trim().is_empty() => pid.to_string(),
        _ => "__shared__".to_string(),
    }
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

/// Result of a traced recall: the injectable text plus the project-db trace id
/// and the chunk ids surfaced, so the outcome can later be written back with
/// `kb_record` to close the feedback loop.
#[derive(Debug, Default, Clone)]
pub struct Recall {
    pub text: String,
    /// Trace id from the *project* db recall (the working memory we calibrate).
    pub trace_id: Option<String>,
    /// Chunk ids surfaced from the project db (the `--used` attribution set).
    pub used_ids: Vec<String>,
}

/// Recall like [`kb_recall`], but over JSON so we capture the project-db
/// `trace_id` + chunk ids for a later [`kb_record`]. Still fans out to the
/// shared db for injection text (its trace is left to the timer/curate sweep).
pub async fn kb_recall_traced(project_id: &str, query: &str) -> Recall {
    let q: String = query.chars().take(500).collect();
    if q.trim().is_empty() {
        return Recall::default();
    }
    let mut out = Recall::default();
    let mut parts = Vec::new();

    if let Some(raw) = run_innate(
        &proj_db(project_id),
        &["recall", &q, "--budget", "3000", "--format", "json", "--source", "sdk"],
        45,
    )
    .await
    {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            out.trace_id = v.get("trace_id").and_then(|t| t.as_str()).map(str::to_string);
            let (text, ids) = render_recall_json(&v);
            out.used_ids = ids;
            if !text.trim().is_empty() {
                parts.push(format!("### 项目经验\n{}", text.trim()));
            }
        }
    }

    if let Some(raw) = run_innate(
        &shared_db(),
        &["recall", &q, "--budget", "2000", "--format", "json", "--source", "sdk"],
        45,
    )
    .await
    {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let (text, _) = render_recall_json(&v);
            if !text.trim().is_empty() {
                parts.push(format!("### 通用技能（跨项目）\n{}", text.trim()));
            }
        }
    }

    out.text = parts.join("\n\n");
    out
}

/// Flatten a `recall --format json` payload into injectable text + chunk ids.
fn render_recall_json(v: &serde_json::Value) -> (String, Vec<String>) {
    let mut lines = Vec::new();
    let mut ids = Vec::new();
    for arr in [v.get("knowledge"), v.get("sparks")].into_iter().flatten() {
        if let Some(items) = arr.as_array() {
            for item in items {
                if let Some(c) = item.get("content").and_then(|c| c.as_str()) {
                    if !c.trim().is_empty() {
                        lines.push(format!("- {}", c.trim()));
                    }
                }
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    (lines.join("\n"), ids)
}

/// Close a recall trace with its real-world outcome (the record half of the
/// loop). `feedback` = Some("up"|"down") reinforces / demotes the used chunks;
/// best-effort. Recorded against the project db (where `trace_id` originated).
pub async fn kb_record(
    project_id: &str,
    trace_id: &str,
    used_ids: &[String],
    outcome: &str,
    feedback: Option<&str>,
) {
    if trace_id.trim().is_empty() {
        return;
    }
    let used = used_ids.join(",");
    let mut args: Vec<&str> = vec![
        "record",
        trace_id,
        "--outcome",
        outcome,
        "--source",
        "sdk",
        // explicit empty `--used` means "known none", which is the correct
        // signal when the recall surfaced nothing.
        "--used",
        &used,
    ];
    if let Some(fb) = feedback {
        args.push("--feedback");
        args.push(fb);
    }
    let _ = run_innate(&proj_db(project_id), &args, 45).await;
}

/// Look up the recall trace linked to a change request, write back its outcome
/// via [`kb_record`], and consume the row. No-op if no trace was stored.
/// Call at terminal outcome points: merge (success) / review_2 reject (failure).
pub async fn consume_trace_outcome(
    db: &crate::db::Db,
    change_request_id: &str,
    outcome: &str,
    feedback: Option<&str>,
) {
    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT project_id, trace_id, used_ids FROM kb_traces WHERE change_request_id=?",
    )
    .bind(change_request_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let Some((project_id, trace_id, used_ids)) = row else { return };
    let ids: Vec<String> = used_ids
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .collect();
    kb_record(&project_id, &trace_id, &ids, outcome, feedback).await;
    let _ = sqlx::query("DELETE FROM kb_traces WHERE change_request_id=?")
        .bind(change_request_id)
        .execute(db)
        .await;
}

/// Capture a knowledge chunk into the project db (best-effort, fire-and-forget safe).
/// `trigger` is the "when to apply" description that drives Innate's trigger vector.
pub async fn kb_add(project_id: &str, content: &str, trigger: &str) {
    let content: String = content.chars().take(4000).collect();
    let trigger: String = trigger.chars().take(300).collect();
    let ok = run_innate(
        &proj_db(project_id),
        &["add", &content, "--kind", "note", "--trigger", &trigger],
        45,
    )
    .await
    .is_some();
    if ok {
        note_capture(Some(project_id));
    }
}

/// Distil + curate a project's accumulated episodic logs into knowledge.
pub async fn kb_evolve(project_id: &str) {
    let _ = run_innate(&proj_db(project_id), &["evolve", "--trigger", "manual"], 120).await;
}

/// Distil + curate the shared (cross-project) craft db.
pub async fn kb_evolve_shared() {
    let _ = run_innate(&shared_db(), &["evolve", "--trigger", "scheduled"], 120).await;
}

// ── Self-growth: event-threshold auto-evolve ───────────────────────────────
//
// Every successful capture bumps an in-process per-scope counter. Once a scope
// accumulates `EVOLVE_EVERY` captures, we fire a background `evolve` (distil +
// curate) for it and reset the counter. This is the "event-triggered" half of
// the self-growth loop; lib.rs setup runs a periodic timer as the backstop so
// low-activity projects still consolidate. `0` disables the event trigger.

static EVOLVE_EVERY: AtomicU32 = AtomicU32::new(8);
static CAPTURE_COUNTS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

/// Set the capture-count threshold that triggers an automatic background evolve.
/// `0` disables event-triggered evolution (timer backstop still applies).
pub fn set_evolve_threshold(n: u32) {
    EVOLVE_EVERY.store(n, Ordering::Relaxed);
}

/// Record a capture against a scope and, if the threshold is reached, spawn a
/// background evolve and reset the counter. Scope `None` (shared) is exempt —
/// the shared craft db is consolidated only on the timer / explicit command.
fn note_capture(project_id: Option<&str>) {
    let Some(pid) = project_id else { return };
    if pid.trim().is_empty() {
        return;
    }
    let threshold = EVOLVE_EVERY.load(Ordering::Relaxed);
    if threshold == 0 {
        return;
    }
    let counts = CAPTURE_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let due = {
        let mut map = counts.lock().unwrap();
        let entry = map.entry(pid.to_string()).or_insert(0);
        *entry += 1;
        if *entry >= threshold {
            *entry = 0;
            true
        } else {
            false
        }
    };
    if due {
        let pid = pid.to_string();
        tokio::spawn(async move {
            kb_evolve(&pid).await;
        });
    }
}

// ── Command-layer helpers (manual /commands in the meeting room) ────────────
//
// These return a human-readable status `Result` (vs. the fire-and-forget
// pipeline hooks above) so the chat can surface success / "innate unavailable".

async fn run_innate_result(db: &PathBuf, args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let _ = tokio::fs::create_dir_all(kb_base()).await;
    let out = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        Command::new(innate_bin()).arg("--db").arg(db).args(args).output(),
    )
    .await
    .map_err(|_| "Innate 调用超时".to_string())?
    .map_err(|e| format!("无法启动 innate（请确认已安装 innate CLI）：{}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        Err(if err.trim().is_empty() {
            format!("innate 退出码 {:?}", out.status.code())
        } else {
            err.trim().to_string()
        })
    }
}

/// Manually store a knowledge chunk (the `/remember` command).
pub async fn cmd_remember(
    project_id: Option<&str>,
    content: &str,
    trigger: &str,
) -> Result<(), String> {
    let content: String = content.chars().take(4000).collect();
    let trigger: String = trigger.chars().take(300).collect();
    run_innate_result(
        &scope_db(project_id),
        &["add", &content, "--kind", "note", "--trigger", &trigger, "--source", "chat"],
        45,
    )
    .await?;
    note_capture(project_id);
    Ok(())
}

/// Manually recall knowledge (the `/recall` command). Returns injectable text.
pub async fn cmd_recall(project_id: Option<&str>, query: &str) -> Result<String, String> {
    let q: String = query.chars().take(500).collect();
    if q.trim().is_empty() {
        return Err("请在 /recall 后输入要召回的关键词".to_string());
    }
    run_innate_result(
        &scope_db(project_id),
        &["recall", &q, "--budget", "4000", "--include-sparks"],
        45,
    )
    .await
    .map(|t| t.trim().to_string())
}

/// Manually distil + curate a scope (the `/evolve` command).
pub async fn cmd_evolve(project_id: Option<&str>) -> Result<String, String> {
    run_innate_result(&scope_db(project_id), &["evolve", "--trigger", "manual"], 180)
        .await
        .map(|t| t.trim().to_string())
}

/// Knowledge-base health summary for a scope (the `/innate` command).
pub async fn cmd_inspect(project_id: Option<&str>) -> Result<String, String> {
    run_innate_result(&scope_db(project_id), &["inspect"], 45)
        .await
        .map(|t| t.trim().to_string())
}

/// Reset the in-process capture counter for a scope (used after a manual evolve).
pub fn reset_capture_count(project_id: Option<&str>) {
    if let Some(counts) = CAPTURE_COUNTS.get() {
        counts.lock().unwrap().remove(&scope_key(project_id));
    }
}
