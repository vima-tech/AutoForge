use crate::models::agent::{Agent, CreateAgent, UpdateAgent};
use crate::models::llm_config::{CreateLlmConfig, LlmConfig, UpdateLlmConfig};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::State;
use uuid::Uuid;

// ---- LLM Configs ----

/// Never ship the raw API key to the webview. The key stays in SQLite and is
/// read backend-side when calling providers; the UI only learns whether one is
/// set. The frontend sends `api_key` back only when the user actually edits it,
/// so blanking it here does not clobber the stored value on unrelated saves.
fn mask_key(mut cfg: LlmConfig) -> LlmConfig {
    cfg.api_key = String::new();
    cfg
}

/// 归一化接口规范，目前仅支持 openai、anthropic 两种 HTTP wire 格式，其余一律回退 openai。
/// api_spec 是接口规范的唯一真源（provider 字段已移除）。
fn normalize_api_spec(explicit: Option<&str>) -> &'static str {
    match explicit.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("anthropic") => "anthropic",
        _ => "openai",
    }
}

#[tauri::command]
pub async fn list_llm_configs(state: State<'_, AppState>) -> Result<Vec<LlmConfig>, String> {
    let rows = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs ORDER BY created_at")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(mask_key).collect())
}

#[tauri::command]
pub async fn create_llm_config(
    payload: CreateLlmConfig,
    state: State<'_, AppState>,
) -> Result<LlmConfig, String> {
    let id = Uuid::new_v4().to_string();
    let temperature = payload.temperature.unwrap_or(0.3);
    let api_spec = normalize_api_spec(payload.api_spec.as_deref());
    // 上下文窗口不再手填：后端按模型查表 + 接口探测自动推断（见 core::ctx_window）。
    // 此处用明文 key 直接探测；拿不到回退「未知」，后续可在设置页点「重新检测」。
    let ctx_window = crate::core::ctx_window::infer_window(
        &payload.endpoint,
        api_spec,
        &payload.model,
        &payload.api_key,
    )
    .await;
    // 密钥静态加密落库（见 core::secrets）。
    let api_key = crate::core::secrets::encrypt_field(&payload.api_key)?;

    // 多模态能力不再手动开关：后端按模型名自动推断（见 core::vision）。
    let supports_vision = crate::core::vision::supports_vision(&payload.model);

    sqlx::query(
        "INSERT INTO llm_configs (id, name, model, endpoint, api_key, ctx_window, temperature, api_spec, supports_vision)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.model)
    .bind(&payload.endpoint)
    .bind(&api_key)
    .bind(&ctx_window)
    .bind(temperature)
    .bind(api_spec)
    .bind(supports_vision)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map(mask_key)
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
        values.push(crate::core::secrets::encrypt_field(v)?);
    }
    if let Some(v) = payload.temperature {
        sets.push("temperature=?");
        values.push(v.to_string());
    }
    if let Some(v) = payload.enabled {
        sets.push("enabled=?");
        values.push(if v { "1" } else { "0" }.to_string());
    }
    if let Some(ref v) = payload.api_spec {
        sets.push("api_spec=?");
        values.push(normalize_api_spec(Some(v)).to_string());
    }

    // model 变了 → 多模态能力随之改变，按模型名自动重新推断并落库（用户不再手动开关）。
    if let Some(ref model) = payload.model {
        let vision = crate::core::vision::supports_vision(model);
        sets.push("supports_vision=?");
        values.push(if vision { "1" } else { "0" }.to_string());
    }

    // model / endpoint / api_spec 变了，窗口随之改变 → 自动重新推断并落库
    // （用户不再手填）。基于「草稿覆盖落库值」的有效配置探测。
    if payload.model.is_some() || payload.endpoint.is_some() || payload.api_spec.is_some() {
        if let Ok(window) = recompute_ctx_window(&state, &id, &payload).await {
            sets.push("ctx_window=?");
            values.push(window);
        }
    }

    if sets.is_empty() {
        return sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map(mask_key)
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
        .map(mask_key)
        .map_err(|e| e.to_string())
}

/// 以「落库值 + 草稿覆盖」为有效配置，自动推断上下文窗口展示串。
/// 草稿（前台未保存输入）优先，未提供的字段回退落库值；密钥落库为密文需解密。
async fn recompute_ctx_window(
    state: &AppState,
    id: &str,
    draft: &UpdateLlmConfig,
) -> Result<String, String> {
    let cfg = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("LLM 配置不存在: {}", id))?;

    let model = draft.model.clone().unwrap_or(cfg.model);
    let endpoint = draft.endpoint.clone().unwrap_or(cfg.endpoint);
    let api_spec = normalize_api_spec(
        draft
            .api_spec
            .as_deref()
            .or(Some(cfg.api_spec.as_str())),
    );
    // 草稿密钥是明文；落库密钥需解密。缺 key 也照常探测（部分本地部署免鉴权）。
    let api_key = match &draft.api_key {
        Some(k) => k.clone(),
        None => crate::core::secrets::decrypt(&cfg.api_key).unwrap_or_default(),
    };
    Ok(crate::core::ctx_window::infer_window(&endpoint, api_spec, &model, &api_key).await)
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

/// 测试连接结果：连接状态文案 + 顺带刷新的（脱敏）配置。
/// 测试连接时会按当前（含草稿）配置重新推断上下文窗口与多模态能力并落库，
/// 前端据 `config` 即时刷新展示。
#[derive(Debug, Clone, Serialize)]
pub struct TestLlmResult {
    pub message: String,
    pub config: LlmConfig,
}

#[tauri::command]
pub async fn test_llm_connection(
    id: String,
    draft: Option<UpdateLlmConfig>,
    state: State<'_, AppState>,
) -> Result<TestLlmResult, String> {
    let mut cfg = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("LLM 配置不存在: {}", id))?;

    // 落库值为密文，解密后再用；草稿值是前台明文，会在下方覆盖。
    cfg.api_key = crate::core::secrets::decrypt(&cfg.api_key)?;

    // 用前台尚未保存的草稿覆盖落库值，使「填写后立即测试」反映最新输入，
    // 而非已保存的旧数据。
    if let Some(d) = draft {
        if let Some(v) = d.name {
            cfg.name = v;
        }
        if let Some(v) = d.model {
            cfg.model = v;
        }
        if let Some(v) = d.endpoint {
            cfg.endpoint = v;
        }
        if let Some(v) = d.api_key {
            cfg.api_key = v;
        }
        if let Some(v) = d.temperature {
            cfg.temperature = v;
        }
        if let Some(v) = d.enabled {
            cfg.enabled = v;
        }
        if let Some(v) = d.api_spec {
            cfg.api_spec = v;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let api_spec_lc = cfg.api_spec.to_ascii_lowercase();
    let endpoint = cfg.endpoint.trim_end_matches('/');
    let api_spec = normalize_api_spec(Some(&cfg.api_spec));

    // 文本 ping（连通性）。自带计时，使 message 报告真实 ping 延迟而非总耗时。
    let ping_fut = async {
        let started = Instant::now();
        let r = if api_spec_lc == "anthropic" {
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
        };
        (r, started.elapsed().as_millis())
    };

    // 三个独立网络请求并发执行（原本串行，总耗时是三者之和）：
    //   ① 文本 ping（连通性）  ② 上下文窗口探测（/v1/models）  ③ 多模态发图探测
    // 总耗时收敛为三者中的最大值。多模态探测无条件并发，但其结果仅在文本连通时采信
    // （见下），以便把「图片被拒」与鉴权/网络问题区分开；连不通时这次探测被丢弃。
    let (
        (send_result, latency_ms),
        ctx_window,
        vision_probe,
    ) = tokio::join!(
        ping_fut,
        crate::core::ctx_window::infer_window(&cfg.endpoint, api_spec, &cfg.model, &cfg.api_key),
        probe_vision(&client, &api_spec_lc, endpoint, &cfg.model, &cfg.api_key, cfg.temperature),
    );

    // 连接状态文案：成功/HTTP 错误/网络错误都转成 message，不提前返回。
    // ping_ok 记录文本连通性，用于决定是否采信多模态探测结果。
    let (message, ping_ok) = match send_result {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                (format!("连接成功 · {}ms", latency_ms), true)
            } else {
                let body = resp.text().await.unwrap_or_default();
                let detail = body.chars().take(300).collect::<String>();
                (format!("连接失败 · HTTP {} · {}", status, detail), false)
            }
        }
        Err(e) => (format!("连接失败: {}", e), false),
    };

    // 多模态：仅在文本已连通时采信发图探测结果（这是识别 mimo 这类「名字看不出」
    // 模型的唯一可靠途径）。结果不确定（鉴权/网络/服务端错误）或文本不通时维持原值。
    let supports_vision = if ping_ok {
        vision_probe.unwrap_or(cfg.supports_vision)
    } else {
        cfg.supports_vision
    };

    // 仅当行内 model 仍是本次测试所用模型时才落库——避免测试期间用户并发保存/
    // 切换模型，导致这条用旧模型推断出的窗口/多模态结果覆盖最新数据。
    // （切页不影响准确性：DB 为真源，返回 Settings 页会 listLlmConfigs 重新拉取。）
    let tested_model = cfg.model.clone();
    sqlx::query("UPDATE llm_configs SET ctx_window=?, supports_vision=? WHERE id=? AND model=?")
        .bind(&ctx_window)
        .bind(supports_vision)
        .bind(&id)
        .bind(&tested_model)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // 返回本次实测计算值（即便因并发/草稿未落库，也能让 UI 即时回显所测结果）；
    // 其余字段取脱敏后的当前行。
    let mut config = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map(mask_key)
        .map_err(|e| e.to_string())?;
    config.ctx_window = ctx_window;
    config.supports_vision = supports_vision;

    Ok(TestLlmResult { message, config })
}

/// 测试连接时的多模态「读回式」探测：发一张含**随机 4 位数字**的图片，要求模型只回数字，
/// 再校验回复是否含该数字。仅在文本 ping 已连通时调用。
///
/// 为何不能只看 HTTP 200：很多 OpenAI 兼容网关（如 mimo）对带 `image_url` 的请求照常返回
/// 200，却把图片 part 静默丢弃，纯文本模型也会"答得很顺"——只看状态码会把它误判为支持多模态，
/// 实战发图时图被丢、模型答"看不到"。读回式校验把"端点接受图像字段"与"模型真的看见了"区分开。
///
/// 返回 Some(true)=确实读出了数字（支持）/ Some(false)=连通但读不出（不支持或端点丢图）/
/// None=结果不确定（鉴权/限流/服务端/网络错误，调用方保持原值）。
async fn probe_vision(
    client: &reqwest::Client,
    api_spec_lc: &str,
    endpoint: &str,
    model: &str,
    api_key: &str,
    temperature: f64,
) -> Option<bool> {
    use base64::{engine::general_purpose, Engine as _};

    // 随机 4 位数字（首位非 0，避免模型省略前导零）；蒙对概率约 1/9000，足够低。
    let code = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        format!("{}{:03}", rng.gen_range(1..=9), rng.gen_range(0..1000))
    };
    let img_b64 = general_purpose::STANDARD.encode(crate::core::vision::render_code_png(&code));
    let instruction =
        "图片里有一个数字，请只输出这个数字本身，不要包含任何其他文字、空格或标点。";

    let send = if api_spec_lc == "anthropic" {
        client
            .post(join_endpoint(endpoint, "/v1/messages"))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 16,
                "temperature": temperature,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": instruction},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": img_b64}}
                ]}]
            }))
            .send()
            .await
    } else {
        client
            .post(join_endpoint(endpoint, "/v1/chat/completions"))
            .bearer_auth(api_key)
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 16,
                "temperature": 0,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": instruction},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", img_b64)}}
                ]}]
            }))
            .send()
            .await
    };

    match send {
        Ok(resp) => {
            let s = resp.status();
            if s.as_u16() == 401 || s.as_u16() == 403 || s.as_u16() == 429 || s.is_server_error() {
                return None; // 鉴权/限流/服务端错误：与图像能力无关，结果不确定
            }
            if !s.is_success() {
                // 含图请求 4xx（多为 400/415/422）→ 端点直接拒收图像 → 不支持。
                return Some(false);
            }
            let body = resp.json::<serde_json::Value>().await.ok()?;
            let reply = extract_reply_text(api_spec_lc, &body);
            // 只保留数字后判断是否读回了目标 code（substring 容忍模型加前后缀）。
            let digits: String = reply.chars().filter(|c| c.is_ascii_digit()).collect();
            Some(digits.contains(&code))
        }
        Err(_) => None, // 网络层错误：不确定
    }
}

/// 从 LLM 响应体抽取纯文本回复（openai: choices[0].message.content；anthropic: content[].text）。
fn extract_reply_text(api_spec_lc: &str, body: &serde_json::Value) -> String {
    if api_spec_lc == "anthropic" {
        body.get("content")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|it| it.get("text").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    } else {
        body.pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
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
    .bind(role_type)
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
    pub group: String,   // orchestration | delivery | pipeline | knowledge
    pub binding: String, // system_kind | forge_role
    pub desc: String,
    pub usage: String,
    pub color: String,
    pub icon: String,
    pub builtin_prompt: String,
    /// 仅绑定 LLM 的角色（知识层）：前端只渲染 LLM 选择器，隐藏 prompt/群聊等开关。
    pub llm_only: bool,
    pub holder: Option<Agent>,
}

fn group_str(g: crate::agents::roles::RoleGroup) -> &'static str {
    use crate::agents::roles::RoleGroup::*;
    match g {
        Orchestration => "orchestration",
        Delivery => "delivery",
        Pipeline => "pipeline",
        Knowledge => "knowledge",
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
            usage: def.usage.to_string(),
            color: def.color.to_string(),
            icon: def.icon.to_string(),
            builtin_prompt: def.builtin_prompt.to_string(),
            llm_only: def.llm_only,
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
    /// 工具白名单 JSON（约定 `{"tools":[...]}`）；None=不改。
    pub capabilities_json: Option<String>,
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
    if let Some(ref c) = payload.capabilities_json {
        sets.push("capabilities_json=?".to_string());
        vals.push(AgentUpdateValue::Text(c.clone()));
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

    // 知识层蒸馏 LLM 改动后，刷新 in-process Innate 模型配置（best-effort，不阻断保存）。
    if kind == "kb_distill" {
        crate::knowledge::refresh_kb_models(&state.db).await;
    }

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

// ---- 凭据加密后端状态 ----

/// 当前密钥主密钥所在后端："keychain"（系统钥匙环）或 "file"（0600 文件兜底）。
/// 供设置页提示用户兜底场景下安全性较弱。
#[tauri::command]
pub fn secret_backend_status() -> String {
    match crate::core::secrets::backend() {
        crate::core::secrets::SecretBackend::Keychain => "keychain".to_string(),
        crate::core::secrets::SecretBackend::File => "file".to_string(),
    }
}

// ---- 工具：内置工具目录（供前端动态渲染 Agent 能力开关，新增工具自动出现）----

/// 返回内置工具目录的元信息（name/label/needs_project）。前端据此渲染能力开关，
/// 不必硬编码工具清单——在 `agents::tools::builtin_catalog` 加一个工具即自动出现在 UI。
#[tauri::command]
pub fn list_builtin_tools() -> Vec<crate::agents::tools::ToolInfo> {
    crate::agents::tools::builtin_catalog_meta()
}

// ---- 工具：Web 搜索配置（存 app_settings，供 agents/tools/web_search 读取）----

/// 回传给前端的 Web 搜索配置。`api_key` 永不出库到 webview，仅暴露是否已设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchSettings {
    pub provider: String,
    pub endpoint: String,
    pub max_results: u32,
    pub api_key_set: bool,
    /// 是否默认在搜索后自动抓取前几条结果的正文摘录。
    pub fetch_content: bool,
}

async fn read_setting(state: &AppState, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

async fn write_setting(state: &AppState, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---- 操作者身份卡（rail-me）：单操作者桌面端，存 app_settings 的 operator_profile KV ----

/// 操作者（Human-Lite-in-the-Loop 的「人」）身份。无登录/多用户概念。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorProfile {
    /// 显示名（群聊/直聊里人类发言的作者名）。
    pub display_name: String,
    /// 头像内容：emoji 或 1–2 个字符的首字母缩写。
    pub avatar: String,
    /// 强调色（CSS 颜色串）；空串表示沿用主题默认 `--me-avatar-bg`。
    pub accent_color: String,
    /// 角色/头衔（可选，注入编排上下文供 Agent 称呼）。
    pub role: String,
}

impl Default for OperatorProfile {
    fn default() -> Self {
        OperatorProfile {
            display_name: "我".into(),
            avatar: "管".into(),
            accent_color: String::new(),
            role: "操作者".into(),
        }
    }
}

async fn load_operator_profile(state: &AppState) -> OperatorProfile {
    match read_setting(state, "operator_profile").await {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => OperatorProfile::default(),
    }
}

#[tauri::command]
pub async fn get_operator_profile(state: State<'_, AppState>) -> Result<OperatorProfile, String> {
    Ok(load_operator_profile(&state).await)
}

#[tauri::command]
pub async fn set_operator_profile(
    state: State<'_, AppState>,
    profile: OperatorProfile,
) -> Result<OperatorProfile, String> {
    // 归一：去空白；显示名/头像兜底，避免渲染出空头像或空作者名。
    let mut p = profile;
    p.display_name = p.display_name.trim().to_string();
    if p.display_name.is_empty() {
        p.display_name = OperatorProfile::default().display_name;
    }
    p.avatar = p.avatar.trim().chars().take(2).collect();
    if p.avatar.is_empty() {
        p.avatar = p.display_name.chars().next().map(|c| c.to_string()).unwrap_or_default();
    }
    p.accent_color = p.accent_color.trim().to_string();
    p.role = p.role.trim().to_string();

    let json = serde_json::to_string(&p).map_err(|e| e.to_string())?;
    write_setting(&state, "operator_profile", &json).await?;
    Ok(p)
}

#[tauri::command]
pub async fn get_web_search_settings(
    state: State<'_, AppState>,
) -> Result<WebSearchSettings, String> {
    let provider = read_setting(&state, "web_search.provider")
        .await
        .unwrap_or_default();
    let endpoint = read_setting(&state, "web_search.endpoint")
        .await
        .unwrap_or_default();
    let max_results = read_setting(&state, "web_search.max_results")
        .await
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(5);
    let api_key_set = read_setting(&state, "web_search.api_key")
        .await
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let fetch_content = read_setting(&state, "web_search.fetch_content")
        .await
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false);
    Ok(WebSearchSettings {
        provider,
        endpoint,
        max_results,
        api_key_set,
        fetch_content,
    })
}

/// 保存 Web 搜索配置。`api_key` 为 None 时不改动已存的 key（与 LLM 配置一致的语义）。
#[tauri::command]
pub async fn set_web_search_settings(
    provider: String,
    endpoint: String,
    max_results: u32,
    api_key: Option<String>,
    fetch_content: bool,
    state: State<'_, AppState>,
) -> Result<WebSearchSettings, String> {
    write_setting(&state, "web_search.provider", provider.trim()).await?;
    write_setting(&state, "web_search.endpoint", endpoint.trim()).await?;
    write_setting(
        &state,
        "web_search.max_results",
        &max_results.clamp(1, 10).to_string(),
    )
    .await?;
    write_setting(
        &state,
        "web_search.fetch_content",
        if fetch_content { "1" } else { "0" },
    )
    .await?;
    if let Some(key) = api_key {
        let enc = crate::core::secrets::encrypt_field(key.trim())?;
        write_setting(&state, "web_search.api_key", &enc).await?;
    }
    get_web_search_settings(state).await
}

// ---- 语音录入（ASR）配置（存 app_settings，供 agents/asr 读取）----

/// 回传给前端的 ASR 配置。`api_key` 永不出库到 webview，仅暴露是否已设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub language: String,
    pub api_key_set: bool,
}

#[tauri::command]
pub async fn get_asr_settings(state: State<'_, AppState>) -> Result<AsrSettings, String> {
    let provider = read_setting(&state, "asr.provider").await.unwrap_or_default();
    let endpoint = read_setting(&state, "asr.endpoint").await.unwrap_or_default();
    let model = read_setting(&state, "asr.model").await.unwrap_or_default();
    let language = read_setting(&state, "asr.language").await.unwrap_or_default();
    let api_key_set = read_setting(&state, "asr.api_key")
        .await
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    Ok(AsrSettings {
        provider,
        endpoint,
        model,
        language,
        api_key_set,
    })
}

/// 保存 ASR 配置。`api_key` 为 None 时不改动已存的 key（与 LLM/web_search 一致）。
#[tauri::command]
pub async fn set_asr_settings(
    provider: String,
    endpoint: String,
    model: String,
    language: String,
    api_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<AsrSettings, String> {
    write_setting(&state, "asr.provider", provider.trim()).await?;
    write_setting(&state, "asr.endpoint", endpoint.trim()).await?;
    write_setting(&state, "asr.model", model.trim()).await?;
    write_setting(&state, "asr.language", language.trim()).await?;
    if let Some(key) = api_key {
        let enc = crate::core::secrets::encrypt_field(key.trim())?;
        write_setting(&state, "asr.api_key", &enc).await?;
    }
    get_asr_settings(state).await
}

// ---- 工厂自喂料（autosupply）配置（存 app_settings，供调度器读取）----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutosupplySettings {
    pub enabled: bool,
    pub interval_min: i64,
    pub scan_enabled: bool,
    pub proposer_enabled: bool,
    pub max_per_run: i64,
    pub proposer_max_per_run: i64,
    pub min_severity: String,
    pub analyze_enabled: bool,
    pub triage_enabled: bool,
}

#[tauri::command]
pub async fn get_autosupply_settings(
    state: State<'_, AppState>,
) -> Result<AutosupplySettings, String> {
    let cfg = crate::tasks::autosupply::AutosupplyConfig::load(&state.db).await;
    Ok(AutosupplySettings {
        enabled: cfg.enabled,
        interval_min: cfg.interval_min,
        scan_enabled: cfg.scan_enabled,
        proposer_enabled: cfg.proposer_enabled,
        max_per_run: cfg.max_per_run as i64,
        proposer_max_per_run: cfg.proposer_max_per_run as i64,
        min_severity: cfg.min_severity,
        analyze_enabled: cfg.analyze_enabled,
        triage_enabled: cfg.triage_enabled,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn set_autosupply_settings(
    enabled: bool,
    interval_min: i64,
    scan_enabled: bool,
    proposer_enabled: bool,
    max_per_run: i64,
    proposer_max_per_run: i64,
    min_severity: String,
    analyze_enabled: bool,
    triage_enabled: bool,
    state: State<'_, AppState>,
) -> Result<AutosupplySettings, String> {
    // 严重度门槛白名单校验，非法值退回 low（不过滤）。
    let min_sev = match min_severity.trim().to_ascii_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        _ => "low",
    };
    write_setting(&state, "autosupply.enabled", if enabled { "1" } else { "0" }).await?;
    write_setting(&state, "autosupply.interval_min", &interval_min.max(5).to_string()).await?;
    write_setting(&state, "autosupply.scan_enabled", if scan_enabled { "1" } else { "0" }).await?;
    write_setting(&state, "autosupply.proposer_enabled", if proposer_enabled { "1" } else { "0" }).await?;
    write_setting(&state, "autosupply.max_per_run", &max_per_run.clamp(1, 200).to_string()).await?;
    write_setting(&state, "autosupply.proposer_max_per_run", &proposer_max_per_run.clamp(1, 50).to_string()).await?;
    write_setting(&state, "autosupply.min_severity", min_sev).await?;
    write_setting(&state, "autosupply.analyze_enabled", if analyze_enabled { "1" } else { "0" }).await?;
    write_setting(&state, "autosupply.triage_enabled", if triage_enabled { "1" } else { "0" }).await?;
    get_autosupply_settings(state).await
}

// ---- 信任旋钮（autonomy level）：strict / standard / loose ----
// 现阶段对 AI 未完全放心 → 默认 strict。档位应用预设到 autosupply（已被 triage 池闸门兜底），
// **不触碰**审核闸/并发/合并——两道人工审核闸在任何档位都保留（裁决权不下放）。

#[tauri::command]
pub async fn get_autonomy_level(state: State<'_, AppState>) -> Result<String, String> {
    Ok(read_setting(&state, "autonomy.level")
        .await
        .filter(|s| matches!(s.as_str(), "strict" | "standard" | "loose"))
        .unwrap_or_else(|| "strict".to_string()))
}

#[tauri::command]
pub async fn set_autonomy_level(
    level: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let level = match level.as_str() {
        "standard" | "loose" => level,
        _ => "strict".to_string(),
    };
    write_setting(&state, "autonomy.level", &level).await?;
    // 应用预设到自喂料（信任越高，放得越开）。proposer 仅 loose 档自动开。
    let (proposer, max) = match level.as_str() {
        "loose" => ("1", "40"),
        "standard" => ("0", "20"),
        _ => ("0", "10"),
    };
    write_setting(&state, "autosupply.proposer_enabled", proposer).await?;
    write_setting(&state, "autosupply.max_per_run", max).await?;
    Ok(level)
}
