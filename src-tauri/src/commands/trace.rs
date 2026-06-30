//! LLM trace 查询命令：列表（按 trace 汇总）+ 详情（trace 内全部 span）+ 清理。
//! 薄包装：取 state → 调 sqlx → 返回，业务无 Tauri 类型耦合。

use crate::agents::llm::INNATE_RECALL_HEADING;
use crate::models::llm_trace::{LlmTrace, LlmTraceSummary};
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;

/// SQL 片段：判定给定 system_prompt 列是否注入了 Innate 召回小节，产出 0/1 → `innate_triggered`。
/// `col` 为列引用（如 `r.system_prompt` / `system_prompt`）。锚点常量在 `agents/llm.rs`，单一来源；
/// 标题不含单引号，内联进 SQL 安全。
fn innate_expr(col: &str) -> String {
    format!(
        "(COALESCE({col},'') LIKE '%{}%') AS innate_triggered",
        INNATE_RECALL_HEADING
    )
}

/// trace 列表筛选条件（均可选，留空即不限）。
#[derive(Debug, Default, Deserialize)]
pub struct TraceFilter {
    pub issue_id: Option<String>,
    pub conversation_id: Option<String>,
    /// 角色精简名（= agents.name，如「调度器」）。优先用它筛选，比冗长的职责标签更可用。
    pub agent_name: Option<String>,
    pub agent_role: Option<String>,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub status: Option<String>, // ok | error
    pub kind: Option<String>,   // 关键词命中 name/input/output 的简单全文匹配
    pub limit: Option<i64>,
}

fn like(s: &str) -> String {
    format!("%{}%", s.replace('%', "\\%").replace('_', "\\_"))
}

/// 列出 trace（按 trace_id 汇总到 root agent span + 统计 span 数/总 token）。
/// 仅返回有 root(agent) span 的 trace；按时间倒序。
#[tauri::command]
pub async fn list_llm_traces(
    filter: TraceFilter,
    state: State<'_, AppState>,
) -> Result<Vec<LlmTraceSummary>, String> {
    let limit = filter.limit.unwrap_or(200).clamp(1, 1000);

    // root 行汇总；span_count/total_tokens 由子查询按 trace_id 聚合；innate_triggered 由 root 的
    // system_prompt 是否含 Innate 召回小节派生。
    let mut sql = format!(
        "SELECT
            r.trace_id, r.agent_id, r.agent_role, r.agent_name,
            r.project_id, r.issue_id, r.conversation_id, r.task_id,
            r.status, r.error, r.input, r.output, r.latency_ms, r.created_at,
            (SELECT COUNT(*) FROM llm_traces s WHERE s.trace_id = r.trace_id) AS span_count,
            (SELECT COALESCE(SUM(s.total_tokens),0) FROM llm_traces s WHERE s.trace_id = r.trace_id) AS total_tokens,
            {}
         FROM llm_traces r
         WHERE r.kind = 'agent'",
        innate_expr("r.system_prompt"),
    );
    let mut binds: Vec<String> = Vec::new();
    if let Some(v) = filter.issue_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.issue_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.conversation_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.conversation_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.agent_name.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.agent_name = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.agent_role.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.agent_role = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.agent_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.agent_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.project_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.project_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.status.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND r.status = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.kind.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND (r.input LIKE ? ESCAPE '\\' OR r.output LIKE ? ESCAPE '\\' OR r.agent_name LIKE ? ESCAPE '\\')");
        let l = like(v);
        binds.push(l.clone());
        binds.push(l.clone());
        binds.push(l);
    }
    sql.push_str(" ORDER BY r.created_at DESC LIMIT ?");

    let mut q = sqlx::query_as::<_, LlmTraceSummary>(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit);
    q.fetch_all(&state.db).await.map_err(|e| e.to_string())
}

/// 取某 trace 的全部 span（含 root），按 seq 升序（root seq=-1 在最前）。
#[tauri::command]
pub async fn get_llm_trace(
    trace_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<LlmTrace>, String> {
    sqlx::query_as::<_, LlmTrace>(&format!(
        "SELECT *, {} FROM llm_traces WHERE trace_id = ? ORDER BY seq ASC, created_at ASC",
        innate_expr("system_prompt"),
    ))
    .bind(&trace_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 去重的角色精简名列表（= agents.name，如「调度器」；供筛选下拉，比冗长职责标签可用）。
#[tauri::command]
pub async fn list_trace_agent_names(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT agent_name FROM llm_traces WHERE agent_name IS NOT NULL AND agent_name <> '' ORDER BY agent_name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// 清空 trace（可选只清某 trace）。
#[tauri::command]
pub async fn clear_llm_traces(
    trace_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match trace_id.as_deref().filter(|s| !s.is_empty()) {
        Some(tid) => {
            sqlx::query("DELETE FROM llm_traces WHERE trace_id = ?")
                .bind(tid)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        }
        None => {
            sqlx::query("DELETE FROM llm_traces")
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ── LLM 用量与成本视图（#64）────────────────────────────────────────────────
// 把 llm_traces 里 kind='llm' 的 span 按 model 聚合，给出调用次数与 token 消耗，
// 供 Trace 页「用量」Tab 展示整体消耗结构（成本随各模型单价折算，单价前端可配）。

/// 按模型聚合的用量行。
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct LlmUsageRow {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub calls: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

/// LLM 用量统计：按 model 聚合的明细 + 合计。可选时间下限（ISO 字符串，空=全部）。
#[derive(serde::Serialize)]
pub struct LlmUsageStats {
    pub rows: Vec<LlmUsageRow>,
    pub total_calls: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_tokens: i64,
}

#[tauri::command]
pub async fn llm_usage_stats(
    since: Option<String>,
    state: State<'_, AppState>,
) -> Result<LlmUsageStats, String> {
    // since 为空则不过滤；非空按 created_at >= since 过滤（前端传「近7天」等下限）。
    let since_filter = since.filter(|s| !s.trim().is_empty());
    let rows: Vec<LlmUsageRow> = sqlx::query_as(
        "SELECT provider, model,
                COUNT(*) AS calls,
                COALESCE(SUM(prompt_tokens),0)     AS prompt_tokens,
                COALESCE(SUM(completion_tokens),0) AS completion_tokens,
                COALESCE(SUM(total_tokens),0)      AS total_tokens
         FROM llm_traces
         WHERE kind='llm' AND (?1 IS NULL OR created_at >= ?1)
         GROUP BY provider, model
         ORDER BY total_tokens DESC",
    )
    .bind(&since_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut stats = LlmUsageStats {
        total_calls: 0,
        total_prompt_tokens: 0,
        total_completion_tokens: 0,
        total_tokens: 0,
        rows: vec![],
    };
    for r in &rows {
        stats.total_calls += r.calls;
        stats.total_prompt_tokens += r.prompt_tokens;
        stats.total_completion_tokens += r.completion_tokens;
        stats.total_tokens += r.total_tokens;
    }
    stats.rows = rows;
    Ok(stats)
}
