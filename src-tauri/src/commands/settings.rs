use crate::models::agent::{Agent, CreateAgent, UpdateAgent};
use crate::models::llm_config::{CreateLlmConfig, LlmConfig, UpdateLlmConfig};
use crate::state::AppState;
use std::time::{Duration, Instant};
use tauri::State;
use uuid::Uuid;

// ---- LLM Configs ----

#[tauri::command]
pub async fn list_llm_configs(state: State<'_, AppState>) -> Result<Vec<LlmConfig>, String> {
    sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_llm_config(
    payload: CreateLlmConfig,
    state: State<'_, AppState>,
) -> Result<LlmConfig, String> {
    let id = Uuid::new_v4().to_string();
    let ctx_window = payload.ctx_window.unwrap_or_else(|| "200K".to_string());
    let temperature = payload.temperature.unwrap_or(0.3);

    sqlx::query(
        "INSERT INTO llm_configs (id, name, provider, model, endpoint, api_key, ctx_window, temperature)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.provider)
    .bind(&payload.model)
    .bind(&payload.endpoint)
    .bind(&payload.api_key)
    .bind(&ctx_window)
    .bind(temperature)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_llm_config(
    id: String,
    payload: UpdateLlmConfig,
    state: State<'_, AppState>,
) -> Result<LlmConfig, String> {
    let mut sets = vec![];
    let mut values: Vec<String> = vec![];

    if let Some(ref v) = payload.name {
        sets.push("name=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.provider {
        sets.push("provider=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.model {
        sets.push("model=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.endpoint {
        sets.push("endpoint=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.api_key {
        sets.push("api_key=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.ctx_window {
        sets.push("ctx_window=?");
        values.push(v.clone());
    }
    if let Some(v) = payload.temperature {
        sets.push("temperature=?");
        values.push(v.to_string());
    }
    if let Some(v) = payload.enabled {
        sets.push("enabled=?");
        values.push(if v { "1" } else { "0" }.to_string());
    }

    if sets.is_empty() {
        return sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }

    let sql = format!("UPDATE llm_configs SET {} WHERE id=?", sets.join(", "));
    let mut q = sqlx::query(&sql);
    for v in &values {
        q = q.bind(v);
    }
    q.bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_llm_config(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE agents SET llm_id=NULL WHERE llm_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM llm_configs WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn test_llm_connection(id: String, state: State<'_, AppState>) -> Result<String, String> {
    let cfg = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("LLM 配置不存在: {}", id))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let started = Instant::now();
    let provider = cfg.provider.to_ascii_lowercase();
    let endpoint = cfg.endpoint.trim_end_matches('/');

    let response = if provider.contains("ollama") {
        client
            .get(join_endpoint(endpoint, "/api/tags"))
            .send()
            .await
            .map_err(|e| format!("连接失败: {}", e))?
    } else if provider.contains("openai") {
        client
            .post(join_endpoint(endpoint, "/v1/chat/completions"))
            .bearer_auth(&cfg.api_key)
            .json(&serde_json::json!({
                "model": cfg.model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
                "temperature": 0
            }))
            .send()
            .await
            .map_err(|e| format!("连接失败: {}", e))?
    } else {
        client
            .post(join_endpoint(endpoint, "/v1/messages"))
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": cfg.model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
                "temperature": cfg.temperature
            }))
            .send()
            .await
            .map_err(|e| format!("连接失败: {}", e))?
    };

    let latency_ms = started.elapsed().as_millis();
    let status = response.status();
    if status.is_success() {
        Ok(format!("连接成功 · {}ms", latency_ms))
    } else {
        let body = response.text().await.unwrap_or_default();
        let detail = body.chars().take(300).collect::<String>();
        Err(format!("连接失败 · HTTP {} · {}", status, detail))
    }
}

fn join_endpoint(endpoint: &str, path: &str) -> String {
    if endpoint.ends_with(path) {
        endpoint.to_string()
    } else {
        format!("{}{}", endpoint, path)
    }
}

// ---- Agents ----

#[tauri::command]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<Agent>, String> {
    sqlx::query_as::<_, Agent>("SELECT * FROM agents ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_agent(
    payload: CreateAgent,
    state: State<'_, AppState>,
) -> Result<Agent, String> {
    let id = Uuid::new_v4().to_string();
    let name_en = payload.name_en.unwrap_or_default();
    let role = payload.role.unwrap_or_default();
    let color = payload.color.unwrap_or_else(|| "#e8772e".to_string());
    let initial = payload.initial.unwrap_or_else(|| "?".to_string());
    let system_prompt = payload.system_prompt.unwrap_or_default();

    sqlx::query(
        "INSERT INTO agents (id, name, name_en, role, color, initial, llm_id, system_prompt)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&name_en)
    .bind(&role)
    .bind(&color)
    .bind(&initial)
    .bind(&payload.llm_id)
    .bind(&system_prompt)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Agent>("SELECT * FROM agents WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_agent(
    id: String,
    payload: UpdateAgent,
    state: State<'_, AppState>,
) -> Result<Agent, String> {
    let mut sets = vec![];
    let mut values: Vec<String> = vec![];

    if let Some(ref v) = payload.name {
        sets.push("name=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.name_en {
        sets.push("name_en=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.role {
        sets.push("role=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.color {
        sets.push("color=?");
        values.push(v.clone());
    }
    if let Some(ref llm_id) = payload.llm_id {
        match llm_id {
            Some(v) => {
                sets.push("llm_id=?");
                values.push(v.clone());
            }
            None => sets.push("llm_id=NULL"),
        }
    }
    if let Some(ref v) = payload.system_prompt {
        sets.push("system_prompt=?");
        values.push(v.clone());
    }

    // forge_role is Option<Option<String>>: Some(None) means clear the assignment.
    if let Some(ref fr) = payload.forge_role {
        match fr {
            Some(v) => {
                sets.push("forge_role=?");
                values.push(v.clone());
            }
            None => sets.push("forge_role=NULL"),
        }
    }

    if sets.is_empty() {
        return sqlx::query_as::<_, Agent>("SELECT * FROM agents WHERE id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }

    let sql = format!("UPDATE agents SET {} WHERE id=?", sets.join(", "));
    let mut q = sqlx::query(&sql);
    for v in &values {
        q = q.bind(v);
    }
    q.bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Agent>("SELECT * FROM agents WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_agent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM agents WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Atomically clears all agents with `role` then assigns `agent_id` (empty = unassign only).
/// Returns full refreshed agent list — frontend needs no local diff logic.
#[tauri::command]
pub async fn set_agent_forge_role(
    agent_id: String,
    role: String,
    state: State<'_, AppState>,
) -> Result<Vec<Agent>, String> {
    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE agents SET forge_role=NULL WHERE forge_role=?")
        .bind(&role)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if !agent_id.is_empty() {
        sqlx::query("UPDATE agents SET forge_role=? WHERE id=?")
            .bind(&role)
            .bind(&agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as::<_, Agent>("SELECT * FROM agents ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}
