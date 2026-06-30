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

/// 字段级体检结果（schema 驱动 agent 的优化循环载体）。
#[derive(Debug, Default, serde::Serialize)]
pub struct FieldHealth {
    pub role: String,
    /// 本次体检实际聚合的 schema 版本（默认解析为该 role 最新版本）。
    pub schema_version: Option<String>,
    pub total: i64,
    pub status_ok: i64,
    pub status_partial: i64,
    pub status_error: i64,
    /// 各叶子字段的填充率（按 created_at 倒序取样后聚合）。
    pub fields: Vec<FieldStat>,
    /// 该 role 下全部可选 schema 版本及样本量（供 UI 切换；按最新优先）。
    pub versions: Vec<VersionStat>,
}

#[derive(Debug, serde::Serialize)]
pub struct FieldStat {
    pub path: String,
    pub filled: i64,
    pub total: i64,
    pub fill_rate: f64,
}

/// 某 role 下一个 schema 版本的样本量（供版本选择器）。
#[derive(Debug, serde::Serialize)]
pub struct VersionStat {
    pub schema_version: String,
    pub count: i64,
}

/// 递归收集 JSON 的叶子字段路径及其「是否填充」。数组视为叶子（非空即填充），不下钻元素索引避免路径爆炸。
/// 0 / false 视为已填充（模型确实输出了该字段）；null / "" / [] / {} 视为未填充。
fn collect_leaves(prefix: &str, v: &serde_json::Value, out: &mut Vec<(String, bool)>) {
    match v {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                if !prefix.is_empty() {
                    out.push((prefix.to_string(), false));
                }
                return;
            }
            for (k, val) in map {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_leaves(&p, val, out);
            }
        }
        serde_json::Value::Array(a) => out.push((prefix.to_string(), !a.is_empty())),
        serde_json::Value::Null => out.push((prefix.to_string(), false)),
        serde_json::Value::String(s) => out.push((prefix.to_string(), !s.trim().is_empty())),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            out.push((prefix.to_string(), true))
        }
    }
}

/// 某 role（可选版本/项目）的字段级体检：拉取产出在 Rust 内遍历 JSON 统计各字段填充率 + 状态分布。
/// 支撑「字段长期空着 → 暴露 schema/prompt 弱点」的优化循环；不依赖 SQLite JSON 扩展，跨平台稳。
#[tauri::command]
pub async fn agent_output_field_health(
    role: String,
    schema_version: Option<String>,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<FieldHealth, String> {
    let project = project_id.as_deref().filter(|s| !s.is_empty());

    // 先枚举该 role 全部 schema 版本（按最新优先），供 UI 切换 + 默认解析最新版本。
    // 跨版本混算会让 v2 新增字段被 v1 旧样本拉低填充率、误报成弱字段，故体检必须锚定单一版本。
    let mut ver_sql = String::from(
        "SELECT schema_version, COUNT(*) FROM agent_outputs WHERE role = ?",
    );
    let mut ver_binds: Vec<String> = vec![role.clone()];
    if let Some(p) = project {
        ver_sql.push_str(" AND project_id = ?");
        ver_binds.push(p.to_string());
    }
    ver_sql.push_str(" GROUP BY schema_version ORDER BY MAX(created_at) DESC");
    let mut vq = sqlx::query_as::<_, (String, i64)>(&ver_sql);
    for b in &ver_binds {
        vq = vq.bind(b);
    }
    let versions: Vec<VersionStat> = vq
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(schema_version, count)| VersionStat {
            schema_version,
            count,
        })
        .collect();

    // 解析目标版本：显式传入优先，否则取最新（versions 首项）；无任何样本则提前返回空体检。
    let target_version = schema_version
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| versions.first().map(|v| v.schema_version.clone()));
    let target_version = match target_version {
        Some(v) => v,
        None => {
            return Ok(FieldHealth {
                role,
                versions,
                ..Default::default()
            })
        }
    };

    let mut sql =
        String::from("SELECT status, output_json FROM agent_outputs WHERE role = ? AND schema_version = ?");
    let mut binds: Vec<String> = vec![role.clone(), target_version.clone()];
    if let Some(p) = project {
        sql.push_str(" AND project_id = ?");
        binds.push(p.to_string());
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT 1000");

    let mut q = sqlx::query_as::<_, (String, Option<String>)>(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    let mut health = FieldHealth {
        role,
        schema_version: Some(target_version),
        versions,
        ..Default::default()
    };
    // path -> filled 计数；分母统一用样本总数 total（某行缺该字段即记为未填充）。
    let mut filled: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for (status, oj) in &rows {
        health.total += 1;
        match status.as_str() {
            "ok" => health.status_ok += 1,
            "partial" => health.status_partial += 1,
            "error" => health.status_error += 1,
            _ => {}
        }
        if let Some(json) = oj {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
                let mut leaves = Vec::new();
                collect_leaves("", &v, &mut leaves);
                for (path, is_filled) in leaves {
                    let e = filled.entry(path).or_insert(0);
                    if is_filled {
                        *e += 1;
                    }
                }
            }
        }
    }
    let total = health.total.max(1);
    health.fields = filled
        .into_iter()
        .map(|(path, f)| FieldStat {
            path,
            filled: f,
            total: health.total,
            fill_rate: f as f64 / total as f64,
        })
        .collect();
    // 按填充率升序：最该关注的弱字段排在最前。
    health
        .fields
        .sort_by(|a, b| a.fill_rate.partial_cmp(&b.fill_rate).unwrap_or(std::cmp::Ordering::Equal));
    Ok(health)
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
