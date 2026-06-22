//! MCP server 配置的 IPC 命令。命令体保持薄包装（取 state → sqlx → 返回）。
//! env_json / headers_json 可能含密钥：出库到 webview 前对其 value 做掩码（仅暴露 key），
//! 保存时若 value 为空则保留库中旧值（与 LLM api_key 的语义一致）。

use crate::models::mcp_server::{CreateMcpServer, McpServer, UpdateMcpServer};
use crate::state::AppState;
use serde_json::{Map, Value};
use tauri::State;
use uuid::Uuid;

/// 把 JSON 对象字符串里每个 value 掩成空串（保留 key），用于出库展示。
fn mask_map_values(json: &str) -> String {
    match serde_json::from_str::<Map<String, Value>>(json) {
        Ok(map) => {
            let masked: Map<String, Value> = map
                .into_iter()
                .map(|(k, _)| (k, Value::String(String::new())))
                .collect();
            serde_json::to_string(&masked).unwrap_or_else(|_| "{}".to_string())
        }
        Err(_) => "{}".to_string(),
    }
}

/// 合并保存：以 incoming 为准，但 value 为空的 key 沿用 stored 中的旧值（保住未改动的密钥）。
fn merge_secret_map(stored: &str, incoming: &str) -> String {
    let old: Map<String, Value> = serde_json::from_str(stored).unwrap_or_default();
    let new: Map<String, Value> = serde_json::from_str(incoming).unwrap_or_default();
    let mut out = Map::new();
    for (k, v) in new {
        let val = match &v {
            Value::String(s) if s.is_empty() => old.get(&k).cloned().unwrap_or(v),
            _ => v,
        };
        out.insert(k, val);
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "{}".to_string())
}

fn mask(mut s: McpServer) -> McpServer {
    // env/headers 落库为密文：先解密回 JSON，再对 value 掩码出库。
    let env = crate::core::secrets::decrypt(&s.env_json).unwrap_or_default();
    let headers = crate::core::secrets::decrypt(&s.headers_json).unwrap_or_default();
    s.env_json = mask_map_values(&env);
    s.headers_json = mask_map_values(&headers);
    s
}

/// 合并旧密钥（库内密文）与前台明文 incoming，再整体加密落库。
fn merge_and_encrypt(stored_ct: &str, incoming: &str) -> Result<String, String> {
    let stored = crate::core::secrets::decrypt(stored_ct)?;
    let merged = merge_secret_map(&stored, incoming);
    crate::core::secrets::encrypt_field(&merged)
}

#[tauri::command]
pub async fn list_mcp_servers(state: State<'_, AppState>) -> Result<Vec<McpServer>, String> {
    let rows = sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(mask).collect())
}

#[tauri::command]
pub async fn create_mcp_server(
    payload: CreateMcpServer,
    state: State<'_, AppState>,
) -> Result<McpServer, String> {
    let id = Uuid::new_v4().to_string();
    let transport = match payload.transport.as_deref() {
        Some("http") => "http",
        _ => "stdio",
    };
    // env/headers 可能含密钥：整体加密落库（见 core::secrets）。
    let env_json =
        crate::core::secrets::encrypt_field(&payload.env_json.unwrap_or_else(|| "{}".to_string()))?;
    let headers_json = crate::core::secrets::encrypt_field(
        &payload.headers_json.unwrap_or_else(|| "{}".to_string()),
    )?;
    sqlx::query(
        "INSERT INTO mcp_servers
         (id, name, transport, command, args_json, env_json, url, headers_json, agent_ids_json, for_code_agent, capability_map_json, enabled)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(transport)
    .bind(payload.command.unwrap_or_default())
    .bind(payload.args_json.unwrap_or_else(|| "[]".to_string()))
    .bind(&env_json)
    .bind(payload.url.unwrap_or_default())
    .bind(&headers_json)
    .bind(payload.agent_ids_json.unwrap_or_else(|| "[]".to_string()))
    .bind(payload.for_code_agent.unwrap_or(false))
    .bind(payload.capability_map_json.unwrap_or_else(|| "{}".to_string()))
    .bind(payload.enabled.unwrap_or(true))
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    fetch_masked(&state, &id).await
}

#[tauri::command]
pub async fn update_mcp_server(
    id: String,
    payload: UpdateMcpServer,
    state: State<'_, AppState>,
) -> Result<McpServer, String> {
    // 取旧值以便合并 env/headers 的密钥。
    let existing = sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("MCP server 不存在: {}", id))?;

    let mut sets: Vec<&str> = vec![];
    let mut values: Vec<String> = vec![];
    if let Some(v) = payload.name {
        sets.push("name=?");
        values.push(v);
    }
    if let Some(v) = payload.transport {
        sets.push("transport=?");
        values.push(if v == "http" { "http".into() } else { "stdio".into() });
    }
    if let Some(v) = payload.command {
        sets.push("command=?");
        values.push(v);
    }
    if let Some(v) = payload.args_json {
        sets.push("args_json=?");
        values.push(v);
    }
    if let Some(v) = payload.env_json {
        sets.push("env_json=?");
        values.push(merge_and_encrypt(&existing.env_json, &v)?);
    }
    if let Some(v) = payload.url {
        sets.push("url=?");
        values.push(v);
    }
    if let Some(v) = payload.headers_json {
        sets.push("headers_json=?");
        values.push(merge_and_encrypt(&existing.headers_json, &v)?);
    }
    if let Some(v) = payload.agent_ids_json {
        sets.push("agent_ids_json=?");
        values.push(v);
    }
    if let Some(v) = payload.for_code_agent {
        sets.push("for_code_agent=?");
        values.push(if v { "1".into() } else { "0".into() });
    }
    if let Some(v) = payload.capability_map_json {
        sets.push("capability_map_json=?");
        values.push(v);
    }
    if let Some(v) = payload.enabled {
        sets.push("enabled=?");
        values.push(if v { "1".into() } else { "0".into() });
    }

    if !sets.is_empty() {
        let sql = format!("UPDATE mcp_servers SET {} WHERE id=?", sets.join(", "));
        let mut q = sqlx::query(&sql);
        for v in &values {
            q = q.bind(v);
        }
        q.bind(&id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }

    fetch_masked(&state, &id).await
}

#[tauri::command]
pub async fn delete_mcp_server(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM mcp_servers WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 测试连接：用库中（合并草稿后的）配置连上 server 并返回工具名列表。
#[tauri::command]
pub async fn test_mcp_connection(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let server = sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("MCP server 不存在: {}", id))?;
    crate::agents::tools::mcp::test_connection(&server)
        .await
        .map_err(|e| e.to_string())
}

/// 按约定发现指定 code_intel server 的能力映射，返回 capability_map_json 文本（供 UI 回填）。
/// 用库中原始（未掩码、env/headers 仍为密文）配置连接——connect 内部自行解密。
#[tauri::command]
pub async fn discover_code_intel_map(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let server = sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("MCP server 不存在: {}", id))?;
    Ok(crate::agents::tools::code_intel::discover_capability_map(&server).await)
}

async fn fetch_masked(state: &AppState, id: &str) -> Result<McpServer, String> {
    sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers WHERE id=?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map(mask)
        .map_err(|e| e.to_string())
}
