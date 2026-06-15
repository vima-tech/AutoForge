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
    } else if provider.contains("anthropic") {
        // Anthropic native API — distinct /v1/messages format
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
    } else {
        // OpenAI-compatible: covers OpenAI, Azure, 自定义, and any other provider
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
    let base = endpoint.trim_end_matches('/');
    if base.ends_with(path) {
        return base.to_string();
    }
    // Avoid doubling the /v1 prefix when the user pastes a baseURL that already ends with /v1
    // e.g. "https://dashscope.aliyuncs.com/compatible-mode/v1" + "/v1/chat/completions"
    //   → "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
    if base.ends_with("/v1") && path.starts_with("/v1/") {
        return format!("{}{}", base, &path[3..]);
    }
    format!("{}{}", base, path)
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
    let role_type = normalize_role_type(payload.role_type.as_deref());
    let system_kind = payload.system_kind.and_then(normalize_system_kind);
    let capabilities_json = payload
        .capabilities_json
        .unwrap_or_else(|| "[]".to_string());
    let max_concurrency = payload.max_concurrency.unwrap_or(1).clamp(1, 16);
    let visible_in_chat = payload.visible_in_chat.unwrap_or(role_type == "business");
    let mentionable = payload.mentionable.unwrap_or(role_type == "business");
    let enabled = payload.enabled.unwrap_or(true);
    let prompt_mode = normalize_prompt_mode(payload.prompt_mode.as_deref());
    let memory_enabled = payload.memory_enabled.unwrap_or(true);

    sqlx::query(
        "INSERT INTO agents (
            id, name, name_en, role, color, initial, llm_id, system_prompt,
            role_type, system_kind, capabilities_json, max_concurrency,
            visible_in_chat, mentionable, enabled, prompt_mode, memory_enabled
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&name_en)
    .bind(&role)
    .bind(&color)
    .bind(&initial)
    .bind(&payload.llm_id)
    .bind(&system_prompt)
    .bind(&role_type)
    .bind(&system_kind)
    .bind(&capabilities_json)
    .bind(max_concurrency)
    .bind(visible_in_chat)
    .bind(mentionable)
    .bind(enabled)
    .bind(prompt_mode)
    .bind(memory_enabled)
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
    let mut values: Vec<AgentUpdateValue> = vec![];

    if let Some(ref v) = payload.name {
        sets.push("name=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }
    if let Some(ref v) = payload.name_en {
        sets.push("name_en=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }
    if let Some(ref v) = payload.role {
        sets.push("role=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }
    if let Some(ref v) = payload.color {
        sets.push("color=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }
    if let Some(ref llm_id) = payload.llm_id {
        match llm_id {
            Some(v) => {
                sets.push("llm_id=?");
                values.push(AgentUpdateValue::Text(v.clone()));
            }
            None => sets.push("llm_id=NULL"),
        }
    }
    if let Some(ref v) = payload.system_prompt {
        sets.push("system_prompt=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }

    // forge_role is Option<Option<String>>: Some(None) means clear the assignment.
    if let Some(ref fr) = payload.forge_role {
        match fr {
            Some(v) => {
                sets.push("forge_role=?");
                values.push(AgentUpdateValue::Text(v.clone()));
            }
            None => sets.push("forge_role=NULL"),
        }
    }
    if let Some(ref v) = payload.role_type {
        sets.push("role_type=?");
        values.push(AgentUpdateValue::Text(
            normalize_role_type(Some(v)).to_string(),
        ));
    }
    if let Some(ref v) = payload.system_kind {
        match v {
            Some(kind) => {
                sets.push("system_kind=?");
                values.push(AgentUpdateValue::Text(
                    normalize_system_kind(kind.clone()).unwrap_or_else(|| "planner".to_string()),
                ));
            }
            None => sets.push("system_kind=NULL"),
        }
    }
    if let Some(ref v) = payload.capabilities_json {
        sets.push("capabilities_json=?");
        values.push(AgentUpdateValue::Text(v.clone()));
    }
    if let Some(v) = payload.max_concurrency {
        sets.push("max_concurrency=?");
        values.push(AgentUpdateValue::Int(v.clamp(1, 16)));
    }
    if let Some(v) = payload.visible_in_chat {
        sets.push("visible_in_chat=?");
        values.push(AgentUpdateValue::Bool(v));
    }
    if let Some(v) = payload.mentionable {
        sets.push("mentionable=?");
        values.push(AgentUpdateValue::Bool(v));
    }
    if let Some(v) = payload.enabled {
        sets.push("enabled=?");
        values.push(AgentUpdateValue::Bool(v));
    }
    if let Some(ref v) = payload.prompt_mode {
        sets.push("prompt_mode=?");
        values.push(AgentUpdateValue::Text(normalize_prompt_mode(Some(v)).to_string()));
    }
    if let Some(v) = payload.memory_enabled {
        sets.push("memory_enabled=?");
        values.push(AgentUpdateValue::Bool(v));
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
        q = match v {
            AgentUpdateValue::Text(v) => q.bind(v),
            AgentUpdateValue::Int(v) => q.bind(v),
            AgentUpdateValue::Bool(v) => q.bind(v),
        };
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

enum AgentUpdateValue {
    Text(String),
    Int(i64),
    Bool(bool),
}

fn normalize_role_type(value: Option<&str>) -> &'static str {
    match value {
        Some("system") => "system",
        _ => "business",
    }
}

fn normalize_system_kind(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_prompt_mode(value: Option<&str>) -> &'static str {
    match value {
        Some("append") => "append",
        Some("custom") => "custom",
        _ => "builtin",
    }
}

// ---- Role catalog (两层模型：角色即 Agent，内置专业提示词) ----

#[derive(serde::Serialize)]
pub struct RoleSlot {
    pub kind: String,
    pub name: String,
    pub name_en: String,
    pub group: String,   // orchestration | delivery | pipeline
    pub binding: String, // system_kind | forge_role
    pub desc: String,
    pub color: String,
    pub icon: String,
    pub builtin_prompt: String,
    pub holder: Option<Agent>,
}

fn group_str(g: crate::agents::roles::RoleGroup) -> &'static str {
    use crate::agents::roles::RoleGroup::*;
    match g {
        Orchestration => "orchestration",
        Delivery => "delivery",
        Pipeline => "pipeline",
    }
}

fn agent_holds(a: &Agent, binding: crate::agents::roles::RoleBinding, kind: &str) -> bool {
    use crate::agents::roles::RoleBinding::*;
    let field = match binding {
        SystemKind => a.system_kind.as_deref(),
        ForgeRole => a.forge_role.as_deref(),
    };
    field
        .map(|s| s.split(',').any(|k| k.trim() == kind))
        .unwrap_or(false)
}

/// 返回内置角色目录 + 各角色当前持有的 Agent（驱动「角色」页系统角色卡）。
#[tauri::command]
pub async fn list_role_catalog(state: State<'_, AppState>) -> Result<Vec<RoleSlot>, String> {
    let agents = sqlx::query_as::<_, Agent>("SELECT * FROM agents")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for def in crate::agents::roles::registry() {
        let binding = match def.binding {
            crate::agents::roles::RoleBinding::SystemKind => "system_kind",
            crate::agents::roles::RoleBinding::ForgeRole => "forge_role",
        };
        let holder = agents
            .iter()
            .find(|a| agent_holds(a, def.binding, def.kind))
            .cloned();
        out.push(RoleSlot {
            kind: def.kind.to_string(),
            name: def.name.to_string(),
            name_en: def.name_en.to_string(),
            group: group_str(def.group).to_string(),
            binding: binding.to_string(),
            desc: def.desc.to_string(),
            color: def.color.to_string(),
            icon: def.icon.to_string(),
            builtin_prompt: def.builtin_prompt.to_string(),
            holder,
        });
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
pub struct SetRoleSlotPayload {
    /// Some("")=解绑 LLM；Some(id)=设置；None=不改
    pub llm_id: Option<String>,
    pub prompt_mode: Option<String>,
    pub supplement: Option<String>, // 映射到 agents.system_prompt
    pub enabled: Option<bool>,
    pub visible_in_chat: Option<bool>,
    pub mentionable: Option<bool>,
    pub memory_enabled: Option<bool>,
}

/// 配置某个内置角色：自动确保单一持有 Agent（无则按注册表默认创建），并应用配置。
/// 不支持一个 Agent 兼多角色——该 slot 的 Agent 只持有此 kind。
#[tauri::command]
pub async fn set_role_slot(
    kind: String,
    payload: SetRoleSlotPayload,
    state: State<'_, AppState>,
) -> Result<Vec<RoleSlot>, String> {
    let def = *crate::agents::roles::find(&kind).ok_or_else(|| format!("未知角色: {}", kind))?;
    let col = match def.binding {
        crate::agents::roles::RoleBinding::SystemKind => "system_kind",
        crate::agents::roles::RoleBinding::ForgeRole => "forge_role",
    };

    let agents = sqlx::query_as::<_, Agent>("SELECT * FROM agents")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let holders: Vec<&Agent> = agents
        .iter()
        .filter(|a| agent_holds(a, def.binding, &kind))
        .collect();

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

    // 取第一个持有者为该 slot 的 Agent；其余清除该 kind（保证单一持有）。
    let primary_id: String = if let Some(first) = holders.first() {
        for extra in holders.iter().skip(1) {
            let remaining: Vec<&str> = extra
                .system_kind
                .as_deref()
                .or(extra.forge_role.as_deref())
                .unwrap_or("")
                .split(',')
                .filter(|k| k.trim() != kind && !k.trim().is_empty())
                .collect();
            let new_val: Option<String> = if remaining.is_empty() { None } else { Some(remaining.join(",")) };
            sqlx::query(&format!("UPDATE agents SET {col}=? WHERE id=?"))
                .bind(&new_val)
                .bind(&extra.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        first.id.clone()
    } else {
        // 无持有者：按注册表默认创建一个专属 Agent。
        let id = Uuid::new_v4().to_string();
        let role_type = match def.binding {
            crate::agents::roles::RoleBinding::ForgeRole => "business",
            crate::agents::roles::RoleBinding::SystemKind => "system",
        };
        let (sk, fr): (Option<&str>, Option<&str>) = match def.binding {
            crate::agents::roles::RoleBinding::SystemKind => (Some(def.kind), None),
            crate::agents::roles::RoleBinding::ForgeRole => (None, Some(def.kind)),
        };
        sqlx::query(
            "INSERT INTO agents (
                id, name, name_en, role, color, initial, llm_id, system_prompt,
                forge_role, role_type, system_kind, capabilities_json, max_concurrency,
                visible_in_chat, mentionable, enabled, prompt_mode
             ) VALUES (?, ?, ?, ?, ?, ?, NULL, '', ?, ?, ?, ?, 1, ?, ?, 1, 'builtin')",
        )
        .bind(&id)
        .bind(def.name)
        .bind(def.name_en)
        .bind(def.desc)
        .bind(def.color)
        .bind(def.initial)
        .bind(fr)
        .bind(role_type)
        .bind(sk)
        .bind(def.default_caps)
        .bind(def.default_chat)
        .bind(def.default_chat)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        id
    };

    // 应用配置（仅 provided 字段）。
    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<AgentUpdateValue> = Vec::new();
    if let Some(ref llm) = payload.llm_id {
        if llm.trim().is_empty() {
            sets.push("llm_id=NULL".to_string());
        } else {
            sets.push("llm_id=?".to_string());
            vals.push(AgentUpdateValue::Text(llm.clone()));
        }
    }
    if let Some(ref m) = payload.prompt_mode {
        sets.push("prompt_mode=?".to_string());
        vals.push(AgentUpdateValue::Text(normalize_prompt_mode(Some(m)).to_string()));
    }
    if let Some(ref s) = payload.supplement {
        sets.push("system_prompt=?".to_string());
        vals.push(AgentUpdateValue::Text(s.clone()));
    }
    if let Some(v) = payload.enabled {
        sets.push("enabled=?".to_string());
        vals.push(AgentUpdateValue::Bool(v));
    }
    if let Some(v) = payload.visible_in_chat {
        sets.push("visible_in_chat=?".to_string());
        vals.push(AgentUpdateValue::Bool(v));
    }
    if let Some(v) = payload.mentionable {
        sets.push("mentionable=?".to_string());
        vals.push(AgentUpdateValue::Bool(v));
    }
    if let Some(v) = payload.memory_enabled {
        sets.push("memory_enabled=?".to_string());
        vals.push(AgentUpdateValue::Bool(v));
    }
    if !sets.is_empty() {
        let sql = format!("UPDATE agents SET {} WHERE id=?", sets.join(", "));
        let mut q = sqlx::query(&sql);
        for v in &vals {
            q = match v {
                AgentUpdateValue::Text(v) => q.bind(v),
                AgentUpdateValue::Int(v) => q.bind(v),
                AgentUpdateValue::Bool(v) => q.bind(v),
            };
        }
        q.bind(&primary_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    list_role_catalog(state).await
}

#[tauri::command]
pub async fn delete_agent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM conversation_members WHERE agent_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM agents WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}

/// Assigns `role` to `agent_id` (empty = unassign only). forge_role is comma-separated so the
/// same agent can hold both 'analysis' and 'test' simultaneously.
/// Returns full refreshed agent list — frontend needs no local diff logic.
#[tauri::command]
pub async fn set_agent_forge_role(
    agent_id: String,
    role: String,
    state: State<'_, AppState>,
) -> Result<Vec<Agent>, String> {
    // Load current state to compute new comma-separated role lists in Rust.
    let holders = sqlx::query_as::<_, Agent>("SELECT * FROM agents WHERE forge_role IS NOT NULL")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

    // Remove `role` from every agent that currently holds it.
    for a in &holders {
        if let Some(fr) = &a.forge_role {
            if fr.split(',').any(|r| r == role) {
                let remaining: Vec<&str> = fr.split(',').filter(|&r| r != role).collect();
                let new_fr: Option<String> = if remaining.is_empty() {
                    None
                } else {
                    Some(remaining.join(","))
                };
                sqlx::query("UPDATE agents SET forge_role=? WHERE id=?")
                    .bind(&new_fr)
                    .bind(&a.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    // Add `role` to the target agent (merging with its existing roles).
    if !agent_id.is_empty() {
        let existing: Vec<String> = holders
            .iter()
            .find(|a| a.id == agent_id)
            .and_then(|a| a.forge_role.as_deref())
            .map(|fr| {
                fr.split(',')
                    .filter(|&r| r != role)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let mut new_roles = existing;
        new_roles.push(role.clone());
        let new_fr = new_roles.join(",");
        sqlx::query("UPDATE agents SET forge_role=? WHERE id=?")
            .bind(&new_fr)
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
