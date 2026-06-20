//! Innate integration — AutoForge self-growth knowledge layer.
//!
//! Strategy: Innate is embedded **in-process** as the `innate` crate (lib
//! `innate_core`), not shelled out. We construct the LLM/embedding providers
//! from AutoForge's own unified config (the `kb_distill` role's LLM + the
//! embedding settings) and inject them via `KnowledgeBase::open_with`, so Innate
//! never reads the global `~/.innate/settings.json` and has **zero relationship
//! with the outside world** (no CLI, no shared global state, no effect on other
//! agents' Innate MCP). All operations are best-effort: if a provider is
//! unconfigured Innate degrades gracefully (dummy embedder / heuristic
//! distiller); errors never break the pipeline. Tenancy = per-project db + a
//! shared db (shared = the factory's durable craft; proj = working memory).
//!
//! Concurrency: `KnowledgeBase` is sync (rusqlite) and its providers do blocking
//! HTTP (`ureq`), so every operation runs on `tokio::task::spawn_blocking`. Each
//! call opens its own connection (SQLite WAL handles multi-connection access).

use crate::state::kb_base;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use innate_core::embedding::EmbeddingProvider;
use innate_core::llm::{build_distiller, LlmEmbeddingProvider};
use innate_core::refine::Distiller;
use innate_core::settings::{EmbeddingConfig as InEmbeddingConfig, LlmConfig as InLlmConfig};
use innate_core::{KnowledgeBase, RecallParams, RecallResult, RecordParams};

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

// ── In-process model config (Innate providers) ──────────────────────────────
//
// The embedding + distill-LLM config that drive Innate's providers. Cached in a
// global so the per-call knowledge functions stay db-free; refreshed at startup
// and whenever the user saves the knowledge-layer config (`refresh_kb_models`).

#[derive(Default, Clone)]
struct KbModels {
    embedding: Option<InEmbeddingConfig>,
    distill: Option<InLlmConfig>,
}

static KB_MODELS: OnceLock<RwLock<KbModels>> = OnceLock::new();

fn kb_models() -> &'static RwLock<KbModels> {
    KB_MODELS.get_or_init(|| RwLock::new(KbModels::default()))
}

fn set_kb_models(embedding: Option<InEmbeddingConfig>, distill: Option<InLlmConfig>) {
    *kb_models().write().unwrap() = KbModels { embedding, distill };
    // Providers are baked into open `KnowledgeBase` handles at open time, so a
    // config change invalidates the whole handle cache; dim alerts referenced the
    // old config and self-correct on the next open.
    clear_kb_cache();
    if let Some(a) = DIM_ALERTS.get() {
        a.write().unwrap().clear();
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// Map an AutoForge LLM `api_spec` → Innate provider enum (openai | anthropic).
/// `claude-cli` has no HTTP API → `None` (unusable for Innate distillation).
fn innate_provider(api_spec: &str) -> Option<&'static str> {
    let p = api_spec.to_ascii_lowercase();
    if p.contains("claude-cli") {
        None
    } else if p.contains("anthropic") {
        Some("anthropic")
    } else {
        Some("openai")
    }
}

/// Resolve the `llm_configs` row bound to the `kb_distill` role (None if no
/// holder / no LLM bound).
async fn resolve_distill_llm(db: &crate::db::Db) -> Option<crate::models::llm_config::LlmConfig> {
    let agent = sqlx::query_as::<_, crate::models::agent::Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(system_kind, '') || ',') LIKE '%,kb_distill,%'
           AND enabled=1
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;
    let llm_id = agent.llm_id?;
    sqlx::query_as::<_, crate::models::llm_config::LlmConfig>(
        "SELECT * FROM llm_configs WHERE id=?",
    )
    .bind(&llm_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Rebuild the cached Innate model config from AutoForge's unified config.
/// Call at startup and after the user saves the embedding / `kb_distill` config.
pub async fn refresh_kb_models(db: &crate::db::Db) {
    let emb = crate::commands::knowledge::load_embedding_settings(db).await;
    let embedding = if emb.model_id.trim().is_empty() {
        None
    } else {
        Some(InEmbeddingConfig {
            provider: non_empty(&emb.provider).unwrap_or_else(|| "openai".to_string()),
            base_url: non_empty(&emb.base_url),
            model_id: emb.model_id.trim().to_string(),
            api_key: non_empty(&emb.api_key),
            dim: emb.dim as usize,
        })
    };

    let distill = resolve_distill_llm(db).await.and_then(|cfg| {
        let provider = innate_provider(&cfg.api_spec)?; // None for claude-cli
        Some(InLlmConfig {
            provider: provider.to_string(),
            base_url: non_empty(&cfg.endpoint),
            model_id: cfg.model.trim().to_string(),
            api_key: non_empty(&cfg.api_key),
        })
    });

    match &embedding {
        Some(e) => eprintln!(
            "[innate] embedding active: model={} dim={} (real provider)",
            e.model_id, e.dim
        ),
        None => eprintln!(
            "[innate] embedding unconfigured → dummy embedder; recall degrades to heuristic match"
        ),
    }
    set_kb_models(embedding, distill);
}

// ── Open-handle cache (B-1) ─────────────────────────────────────────────────
//
// Opening a `KnowledgeBase` is not free: it opens a SQLite connection, runs
// schema/migration checks + `init_meta`, and bakes in the embedding/distiller
// providers. Recall sits on the hot path of *every* system role (and fans out
// proj ⊕ shared), so re-opening per call is wasteful. We cache one
// `Arc<Mutex<KnowledgeBase>>` per db path — mirroring the Innate MCP server,
// which holds a single `Mutex<KnowledgeBase>` for its whole lifetime.
// `KnowledgeBase` is `Send` but not `Sync` (rusqlite `Connection`), so the
// `Mutex` both makes the handle shareable across `spawn_blocking` threads and
// serializes access to a given db — acceptable here: concurrency is bounded and
// SQLite serializes writers regardless. The cache is dropped on config change
// (`set_kb_models`), since providers are fixed at open time.

type KbHandle = Arc<Mutex<KnowledgeBase>>;
static KB_CACHE: OnceLock<RwLock<HashMap<PathBuf, KbHandle>>> = OnceLock::new();

fn kb_cache() -> &'static RwLock<HashMap<PathBuf, KbHandle>> {
    KB_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn clear_kb_cache() {
    if let Some(c) = KB_CACHE.get() {
        c.write().unwrap().clear();
    }
}

// ── Embedding-dimension health (B-2) ────────────────────────────────────────
//
// Innate already fails closed on a dim change: `open_with` → `init_meta`
// returns "content_dim mismatch: database uses X, embedding provider uses Y"
// when the configured embedding's dimension differs from what the db was built
// with (a silent recall blackout otherwise, since cosine search only scores
// equal-dim vectors). Our best-effort opens would swallow that into `None`. We
// detect that specific error, log it once, and stash a per-db alert so `/innate`
// and the settings health view can show an actionable message instead of
// quietly returning nothing.

static DIM_ALERTS: OnceLock<RwLock<HashMap<PathBuf, String>>> = OnceLock::new();

fn dim_alerts() -> &'static RwLock<HashMap<PathBuf, String>> {
    DIM_ALERTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Open succeeded → clear any stale dim alert for this db.
fn note_open_ok(db: &PathBuf) {
    if let Some(a) = DIM_ALERTS.get() {
        a.write().unwrap().remove(db);
    }
}

/// Open failed → if it's a dimension mismatch, record an actionable alert.
fn note_open_err(db: &PathBuf, err: &str) {
    if err.contains("mismatch") && err.contains("dim") {
        let msg = format!(
            "向量维度与当前 embedding 配置不一致（{}）。更换 embedding 模型后，旧向量会被静默跳过，\
             召回质量将下降。请改回原 embedding 模型，或清空并重建该库（{}）。",
            err,
            db.display()
        );
        eprintln!("[innate] dim-guard: {msg}");
        dim_alerts().write().unwrap().insert(db.clone(), msg);
    }
}

/// Active embedding-dimension alert for a scope, if any (for `/innate` + health).
pub fn kb_dim_alert(project_id: Option<&str>) -> Option<String> {
    let db = scope_db(project_id);
    DIM_ALERTS.get()?.read().unwrap().get(&db).cloned()
}

/// Get the cached `KnowledgeBase` for `db`, opening (and caching) it with the
/// current providers if absent. Opening runs on a blocking thread (sync IO).
async fn cached_kb(db: PathBuf) -> Result<KbHandle, String> {
    if let Some(h) = kb_cache().read().unwrap().get(&db).cloned() {
        return Ok(h);
    }
    let models = kb_models().read().unwrap().clone();
    let db_for_open = db.clone();
    let opened = tokio::task::spawn_blocking(move || -> Result<KnowledgeBase, String> {
        let embedding = models
            .embedding
            .map(|c| Arc::new(LlmEmbeddingProvider::new(c)) as Arc<dyn EmbeddingProvider>);
        let distiller = models
            .distill
            .as_ref()
            .map(|c| build_distiller(c) as Arc<dyn Distiller>);
        KnowledgeBase::open_with(&db_for_open, embedding, None, distiller, None, None)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("知识库任务异常：{e}"))?;

    match opened {
        Ok(kb) => {
            note_open_ok(&db);
            let handle: KbHandle = Arc::new(Mutex::new(kb));
            // Double-checked insert: a racing open may have populated the slot.
            let mut w = kb_cache().write().unwrap();
            Ok(w.entry(db).or_insert(handle).clone())
        }
        Err(e) => {
            note_open_err(&db, &e);
            Err(format!("打开知识库失败：{e}"))
        }
    }
}

/// Run `f` against the cached `KnowledgeBase` at `db` on a blocking thread.
/// `Err(String)` on open/op failure (for the command layer); [`with_kb`] wraps
/// this best-effort for pipeline hooks.
async fn with_kb_result<T, F>(db: PathBuf, f: F) -> Result<T, String>
where
    F: FnOnce(&KnowledgeBase) -> innate_core::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let handle = cached_kb(db).await?;
    tokio::task::spawn_blocking(move || -> Result<T, String> {
        let kb = handle.lock().unwrap();
        f(&kb).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("知识库任务异常：{e}"))?
}

/// Best-effort variant: returns `None` on any error (fire-and-forget safe).
async fn with_kb<T, F>(db: PathBuf, f: F) -> Option<T>
where
    F: FnOnce(&KnowledgeBase) -> innate_core::Result<T> + Send + 'static,
    T: Send + 'static,
{
    with_kb_result(db, f).await.ok()
}

/// Run a recall against one db. `trace` persists a usage trace (for the
/// record/feedback loop); pass `false` for throwaway recalls.
async fn run_recall(
    db: PathBuf,
    query: String,
    budget: usize,
    include_sparks: bool,
    trace: bool,
) -> Option<RecallResult> {
    with_kb(db, move |kb| {
        kb.recall(RecallParams {
            query: &query,
            budget,
            trace,
            include_sparks,
            source: "sdk",
            ..Default::default()
        })
    })
    .await
}

/// Flatten a `RecallResult` into injectable text + the surfaced chunk ids.
fn render_recall(rr: &RecallResult) -> (String, Vec<String>) {
    let mut lines = Vec::new();
    let mut ids = Vec::new();
    for item in rr.knowledge.iter().chain(rr.sparks.iter()) {
        if let Some(c) = item.get("content").and_then(|c| c.as_str()) {
            if !c.trim().is_empty() {
                lines.push(format!("- {}", c.trim()));
            }
        }
        if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
            ids.push(id.to_string());
        }
    }
    (lines.join("\n"), ids)
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
    if let Some(rr) = run_recall(proj_db(project_id), q.clone(), 3000, false, false).await {
        let (text, _) = render_recall(&rr);
        if !text.trim().is_empty() {
            parts.push(format!("### 项目经验\n{}", text.trim()));
        }
    }
    if let Some(rr) = run_recall(shared_db(), q, 2000, false, false).await {
        let (text, _) = render_recall(&rr);
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

/// Recall like [`kb_recall`], but capture the project-db `trace_id` + chunk ids
/// for a later [`kb_record`]. Still fans out to the shared db for injection text
/// (its trace is left to the timer/curate sweep).
pub async fn kb_recall_traced(project_id: &str, query: &str) -> Recall {
    let q: String = query.chars().take(500).collect();
    if q.trim().is_empty() {
        return Recall::default();
    }
    let mut out = Recall::default();
    let mut parts = Vec::new();

    if let Some(rr) = run_recall(proj_db(project_id), q.clone(), 3000, false, true).await {
        out.trace_id = Some(rr.trace_id.clone());
        let (text, ids) = render_recall(&rr);
        out.used_ids = ids;
        if !text.trim().is_empty() {
            parts.push(format!("### 项目经验\n{}", text.trim()));
        }
    }
    if let Some(rr) = run_recall(shared_db(), q, 2000, false, false).await {
        let (text, _) = render_recall(&rr);
        if !text.trim().is_empty() {
            parts.push(format!("### 通用技能（跨项目）\n{}", text.trim()));
        }
    }

    out.text = parts.join("\n\n");
    out
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
    let trace_id = trace_id.to_string();
    let used: Vec<String> = used_ids.to_vec();
    let outcome = outcome.to_string();
    let feedback = feedback.map(str::to_string);
    let _ = with_kb(proj_db(project_id), move |kb| {
        let (feedback_up, feedback_down): (Option<&[String]>, Option<&[String]>) =
            match feedback.as_deref() {
                Some("up") => (Some(&used), None),
                Some("down") => (None, Some(&used)),
                _ => (None, None),
            };
        kb.record(RecordParams {
            trace_id: &trace_id,
            outcome: Some(&outcome),
            // explicit empty `used` means "known none" — correct when the recall
            // surfaced nothing.
            used: Some(&used),
            feedback_up,
            feedback_down,
            source: "sdk",
            ..Default::default()
        })
    })
    .await;
}

/// Persist a recall trace for a pipeline target so its eventual real-world
/// outcome can be written back, closing the calibration loop. `scope_kind` is
/// `"issue"` (analysis recall → consumed at review_1) or `"change_request"`
/// (code recall → consumed at merge / review_2 / terminal execution failure).
/// Upsert keyed by (scope_kind, target_id): latest recall wins. No-op when the
/// recall produced no project-db trace (nothing to calibrate).
pub async fn store_recall_trace(
    db: &crate::db::Db,
    scope_kind: &str,
    target_id: &str,
    project_id: &str,
    recall: &Recall,
) {
    let Some(trace_id) = recall.trace_id.as_deref() else {
        return;
    };
    let _ = sqlx::query(
        "INSERT INTO kb_recall_traces (scope_kind, target_id, project_id, trace_id, used_ids, created_at)
         VALUES (?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(scope_kind, target_id) DO UPDATE SET
             project_id=excluded.project_id, trace_id=excluded.trace_id,
             used_ids=excluded.used_ids, created_at=excluded.created_at",
    )
    .bind(scope_kind)
    .bind(target_id)
    .bind(project_id)
    .bind(trace_id)
    .bind(recall.used_ids.join(","))
    .execute(db)
    .await;
}

/// Look up the recall trace linked to a pipeline target, write back its outcome
/// via [`kb_record`], and consume the row. No-op if no trace was stored.
/// `feedback` = `Some("up"|"down")` reinforces/demotes the used chunks; `None`
/// records the outcome for evolution without touching confidence (use for
/// ambiguous terminal failures). Call at terminal outcome points:
/// review_1 (analysis) / merge / review_2 reject / execution failure (code).
pub async fn consume_recall_trace(
    db: &crate::db::Db,
    scope_kind: &str,
    target_id: &str,
    outcome: &str,
    feedback: Option<&str>,
) {
    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT project_id, trace_id, used_ids FROM kb_recall_traces WHERE scope_kind=? AND target_id=?",
    )
    .bind(scope_kind)
    .bind(target_id)
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
    let _ = sqlx::query("DELETE FROM kb_recall_traces WHERE scope_kind=? AND target_id=?")
        .bind(scope_kind)
        .bind(target_id)
        .execute(db)
        .await;
}

/// Capture a knowledge chunk into the project db (best-effort, fire-and-forget safe).
/// `trigger` is the "when to apply" description that drives Innate's trigger vector.
pub async fn kb_add(project_id: &str, content: &str, trigger: &str) {
    let content: String = content.chars().take(4000).collect();
    let trigger: String = trigger.chars().take(300).collect();
    let ok = with_kb(proj_db(project_id), move |kb| {
        kb.add(&content, "note", Some(&trigger), None, "agent", None)
    })
    .await
    .is_some();
    if ok {
        note_capture(Some(project_id));
    }
}

/// Distil + curate a project's accumulated episodic logs into knowledge.
pub async fn kb_evolve(project_id: &str) {
    let _ = with_kb(proj_db(project_id), |kb| kb.evolve("manual")).await;
}

/// Distil + curate the shared (cross-project) craft db.
pub async fn kb_evolve_shared() {
    let _ = with_kb(shared_db(), |kb| kb.evolve("scheduled")).await;
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
// pipeline hooks above) so the chat can surface success / failure detail.

/// Manually store a knowledge chunk (the `/remember` command).
pub async fn cmd_remember(
    project_id: Option<&str>,
    content: &str,
    trigger: &str,
) -> Result<(), String> {
    let content: String = content.chars().take(4000).collect();
    let trigger: String = trigger.chars().take(300).collect();
    with_kb_result(scope_db(project_id), move |kb| {
        kb.add(&content, "note", Some(&trigger), None, "chat", None)
    })
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
    let rr = with_kb_result(scope_db(project_id), move |kb| {
        kb.recall(RecallParams {
            query: &q,
            budget: 4000,
            include_sparks: true,
            source: "sdk",
            ..Default::default()
        })
    })
    .await?;
    let (text, _) = render_recall(&rr);
    Ok(text.trim().to_string())
}

/// Manually distil + curate a scope (the `/evolve` command).
pub async fn cmd_evolve(project_id: Option<&str>) -> Result<String, String> {
    let report = with_kb_result(scope_db(project_id), |kb| kb.evolve("manual")).await?;
    Ok(render_value(&report))
}

/// Knowledge-base health summary for a scope (the `/innate` command).
/// Prepends an embedding-dimension warning when one is active (and surfaces it
/// even when the mismatch is what made the open fail).
pub async fn cmd_inspect(project_id: Option<&str>) -> Result<String, String> {
    match with_kb_result(scope_db(project_id), |kb| kb.inspect()).await {
        Ok(report) => {
            let body = render_value(&report);
            Ok(match kb_dim_alert(project_id) {
                Some(alert) => format!("⚠️ {alert}\n\n{body}"),
                None => body,
            })
        }
        // A dim mismatch fails the open; show the actionable warning instead of a
        // generic error so the user knows exactly what to fix.
        Err(e) => match kb_dim_alert(project_id) {
            Some(alert) => Ok(format!("⚠️ {alert}")),
            None => Err(e),
        },
    }
}

/// Render an Innate JSON report (`evolve` / `inspect`) into readable text.
fn render_value(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// Reset the in-process capture counter for a scope (used after a manual evolve).
pub fn reset_capture_count(project_id: Option<&str>) {
    if let Some(counts) = CAPTURE_COUNTS.get() {
        counts.lock().unwrap().remove(&scope_key(project_id));
    }
}

#[cfg(test)]
mod sdk_smoke {
    use innate_core::{KnowledgeBase, RecallParams};

    // Proves the in-process Innate SDK runs inside AutoForge's build (default
    // dummy embedder + heuristic distiller; no network, no global files).
    #[test]
    fn in_process_add_recall_inspect_evolve() {
        let dir = std::env::temp_dir().join(format!("af-kb-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = dir.join("t.db");

        let kb = KnowledgeBase::open_with(&db, None, None, None, None, None).expect("open_with");
        let id = kb
            .add("写数据库查询时用 sqlx 而非 rusqlite", "note", Some("写数据库查询"), None, "manual", None)
            .expect("add");
        assert!(!id.is_empty(), "add returns a chunk id");

        let rr = kb
            .recall(RecallParams { query: "数据库查询", budget: 2000, source: "sdk", ..Default::default() })
            .expect("recall");
        assert!(!rr.trace_id.is_empty(), "recall yields a trace id");

        let report = kb.inspect().expect("inspect");
        assert!(report.is_object() || report.is_array(), "inspect returns json");

        kb.evolve("manual").expect("evolve");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
