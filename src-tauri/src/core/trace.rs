//! LLM 调用链路追踪（LangSmith 式）。**纯 Rust、零 Tauri 类型**，仅依赖 `Db`。
//!
//! 模型：一次 Agent 调用 = 一个 `trace_id`，其下：
//! - root span（kind=agent）：整体出入参 + 总耗时 + 状态（由 [`scope_run`] + [`record_root`] 写）；
//! - llm span（kind=llm）：每次模型 HTTP 调用的 messages 出入参 + tokens + 模型 + 耗时；
//! - tool span（kind=tool|mcp）：每次工具调用的 args/result + 耗时。
//!
//! 关联维度（issue/会议室/项目/任务）由调用方用 [`with_tags`] 在外层设置，深层调用自动继承；
//! agent 维度由 [`scope_run`] 从 Agent 取。全部写入 best-effort，失败只记日志、不影响主流程。
//!
//! 上下文传播用 `tokio::task_local`：不污染业务函数签名，也满足后端独立化铁律
//! （事件/追踪是横切关注点，业务逻辑对其无感知）。注意 task_local 不跨 `tokio::spawn`
//! 传播——需在 spawn 后的任务体内设置 tags（编排/直聊/分析任务入口均已设置）。

use crate::db::Db;
use std::future::Future;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// 单条 I/O 文本入库上限，避免超大 prompt 撑爆 trace 表。
const MAX_FIELD_CHARS: usize = 16_000;

/// 关联标签（用于按需求/会议室/项目/任务筛选 trace）。
#[derive(Clone, Default)]
pub struct TraceTags {
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub conversation_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Clone)]
struct TraceRun {
    db: Db,
    trace_id: String,
    tags: TraceTags,
    agent_id: Option<String>,
    agent_role: Option<String>,
    agent_name: Option<String>,
    seq: Arc<AtomicI64>,
}

tokio::task_local! {
    static TAGS: TraceTags;
    static RUN: TraceRun;
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_FIELD_CHARS {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX_FIELD_CHARS).collect();
        format!("{head}\n…[已截断 {} 字符]", s.chars().count() - MAX_FIELD_CHARS)
    }
}

fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn current_tags() -> TraceTags {
    TAGS.try_with(|t| t.clone()).unwrap_or_default()
}

/// 设置关联标签覆盖 `f` 执行期间（best-effort；用于编排/直聊/分析等入口）。
pub async fn with_tags<F: Future>(tags: TraceTags, f: F) -> F::Output {
    TAGS.scope(tags, f).await
}

/// 把 `f` 作为一次「Agent 调用」追踪：建立 trace_id（继承当前 tags + 取 Agent 维度），
/// `f` 内部的 llm/tool span 会自动挂到该 trace 下。root span 需调用方在 `f` 结束后用
/// [`record_root`] 补写（以拿到最终出参/状态/耗时）。
pub async fn scope_run<F: Future>(db: &Db, agent: &crate::models::agent::Agent, f: F) -> F::Output {
    // 已处于某个 trace run 中（如编排把 LLM 调用 + 写文件包进同一 run）→ 复用，
    // 避免新建 trace_id 把同一 Agent 动作拆成多条互不关联的 trace。
    if RUN.try_with(|_| ()).is_ok() {
        return f.await;
    }
    let run = TraceRun {
        db: db.clone(),
        trace_id: Uuid::new_v4().to_string(),
        tags: current_tags(),
        agent_id: nonempty(&agent.id),
        agent_role: nonempty(&agent.role),
        agent_name: nonempty(&agent.name),
        seq: Arc::new(AtomicI64::new(0)),
    };
    RUN.scope(run, f).await
}

fn run() -> Option<TraceRun> {
    RUN.try_with(|r| r.clone()).ok()
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    run: &TraceRun,
    id: &str,
    parent_id: Option<&str>,
    seq: i64,
    kind: &str,
    name: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    system_prompt: Option<&str>,
    status: &str,
    error: Option<&str>,
    input: Option<&str>,
    output: Option<&str>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    latency_ms: Option<i64>,
    metadata_json: Option<&str>,
) {
    let res = sqlx::query(
        "INSERT INTO llm_traces (
            id, trace_id, parent_id, seq, kind, name,
            agent_id, agent_role, agent_name, project_id, issue_id, conversation_id, task_id,
            provider, model, system_prompt, status, error, input, output,
            prompt_tokens, completion_tokens, total_tokens, latency_ms, metadata_json, ended_at
         ) VALUES (?,?,?,?,?,?, ?,?,?,?,?,?,?, ?,?,?,?,?,?,?, ?,?,?,?,?, datetime('now'))",
    )
    .bind(id)
    .bind(&run.trace_id)
    .bind(parent_id)
    .bind(seq)
    .bind(kind)
    .bind(name)
    .bind(run.agent_id.as_deref())
    .bind(run.agent_role.as_deref())
    .bind(run.agent_name.as_deref())
    .bind(run.tags.project_id.as_deref())
    .bind(run.tags.issue_id.as_deref())
    .bind(run.tags.conversation_id.as_deref())
    .bind(run.tags.task_id.as_deref())
    .bind(provider)
    .bind(model)
    .bind(system_prompt.map(truncate))
    .bind(status)
    .bind(error.map(truncate))
    .bind(input.map(truncate))
    .bind(output.map(truncate))
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(total_tokens)
    .bind(latency_ms)
    .bind(metadata_json)
    .execute(&run.db)
    .await;
    if let Err(e) = res {
        eprintln!("[trace] 写入 span 失败（已忽略）：{}", e);
    }
}

/// 写 root（kind=agent）span：整体出入参 + 状态 + 耗时。seq=-1 置顶。
pub async fn record_root(
    input: &str,
    system_prompt: Option<&str>,
    output: &str,
    status: &str,
    error: Option<&str>,
    latency_ms: i64,
) {
    let Some(run) = run() else { return };
    let id = run.trace_id.clone();
    insert(
        &run, &id, None, -1, "agent",
        run.agent_name.clone().as_deref(),
        None, None, system_prompt, status, error,
        Some(input), Some(output),
        None, None, None, Some(latency_ms), None,
    )
    .await;
}

/// 写一次模型 HTTP 调用（kind=llm）span。
#[allow(clippy::too_many_arguments)]
pub async fn record_llm(
    provider: &str,
    model: &str,
    system_prompt: Option<&str>,
    input: &str,
    output: &str,
    status: &str,
    error: Option<&str>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    latency_ms: i64,
    metadata_json: Option<&str>,
) {
    let Some(run) = run() else { return };
    let seq = run.seq.fetch_add(1, Ordering::Relaxed);
    let id = Uuid::new_v4().to_string();
    let trace_id = run.trace_id.clone();
    insert(
        &run, &id, Some(&trace_id), seq, "llm", Some(model),
        Some(provider), Some(model), system_prompt, status, error,
        Some(input), Some(output),
        prompt_tokens, completion_tokens, total_tokens, Some(latency_ms), metadata_json,
    )
    .await;
}

/// 写一次工具调用（kind=tool 或 mcp）span。名以 `mcp__` 前缀者归为 mcp。
pub async fn record_tool(name: &str, input: &str, output: &str, ok: bool, latency_ms: i64) {
    let Some(run) = run() else { return };
    let kind = if name.starts_with("mcp__") { "mcp" } else { "tool" };
    let seq = run.seq.fetch_add(1, Ordering::Relaxed);
    let id = Uuid::new_v4().to_string();
    let trace_id = run.trace_id.clone();
    insert(
        &run, &id, Some(&trace_id), seq, kind, Some(name),
        None, None, None, if ok { "ok" } else { "error" }, None,
        Some(input), Some(output),
        None, None, None, Some(latency_ms), None,
    )
    .await;
}
