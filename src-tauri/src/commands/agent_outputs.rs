//! 环节 Agent 结构化产出（`agent_outputs`）查询命令：列表 + 详情 + 角色枚举 + 清理。
//! 薄包装：取 state → 调 sqlx → 返回，业务无 Tauri 类型耦合。

use crate::models::agent_output::{AgentOutput, AgentOutputSummary};
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;

/// 产出列表筛选条件（均可选，留空即不限）。
#[derive(Debug, Default, Deserialize)]
pub struct AgentOutputFilter {
    pub role: Option<String>,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub project_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}

/// 列出环节产出（省去 raw，按时间倒序）。
#[tauri::command]
pub async fn list_agent_outputs(
    filter: AgentOutputFilter,
    state: State<'_, AppState>,
) -> Result<Vec<AgentOutputSummary>, String> {
    let limit = filter.limit.unwrap_or(200).clamp(1, 1000);

    let mut sql = String::from(
        "SELECT id, role, schema_version, target_kind, target_id, project_id, trace_id,
                status, output_json, created_at
         FROM agent_outputs WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();
    if let Some(v) = filter.role.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND role = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.target_kind.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND target_kind = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.target_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND target_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.project_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND project_id = ?");
        binds.push(v.to_string());
    }
    if let Some(v) = filter.status.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND status = ?");
        binds.push(v.to_string());
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");

    let mut q = sqlx::query_as::<_, AgentOutputSummary>(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit);
    q.fetch_all(&state.db).await.map_err(|e| e.to_string())
}

/// 取单条产出的完整行（含 raw）。
#[tauri::command]
pub async fn get_agent_output(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<AgentOutput>, String> {
    sqlx::query_as::<_, AgentOutput>("SELECT * FROM agent_outputs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 去重的角色列表（供筛选下拉）。
#[tauri::command]
pub async fn list_agent_output_roles(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT role FROM agent_outputs ORDER BY role")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// 清空产出（可选只清某条）。
#[tauri::command]
pub async fn clear_agent_outputs(
    id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match id.as_deref().filter(|s| !s.is_empty()) {
        Some(oid) => {
            sqlx::query("DELETE FROM agent_outputs WHERE id = ?")
                .bind(oid)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        }
        None => {
            sqlx::query("DELETE FROM agent_outputs")
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
