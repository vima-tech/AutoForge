use crate::agents::tools::ToolRegistry;
use crate::models::agent::Agent;
use crate::models::llm_config::LlmConfig;
use anyhow::{anyhow, Result};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// 工具循环最大轮数，防止模型无限调用工具。每轮 = 一次模型调用 + 一批工具执行。
const MAX_TOOL_ITERS: usize = 6;

pub async fn run_agent_text(
    db: &crate::db::Db,
    agent: &Agent,
    prompt: &str,
    system_prompt: Option<&str>,
    image_paths: &[PathBuf],
) -> Result<String> {
    let Some(llm_id) = &agent.llm_id else {
        return crate::agents::local_claude::run_text_with_images(
            prompt,
            system_prompt,
            image_paths,
        )
        .await;
    };

    let cfg = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(llm_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow!("LLM 配置不存在: {}", llm_id))?;

    if !cfg.enabled {
        return Err(anyhow!("LLM 配置已禁用: {}", cfg.name));
    }

    if !image_paths.is_empty() {
        return Err(anyhow!(
            "当前 LLM 适配器暂不支持图片输入，请改用 Claude CLI 或移除图片附件"
        ));
    }

    // 接口规范唯一真源：仅 openai、anthropic 两种 wire 格式，其余按 openai 处理。
    match cfg.api_spec.to_ascii_lowercase().as_str() {
        "anthropic" => run_anthropic(&cfg, prompt, system_prompt).await,
        _ => run_openai_compatible(&cfg, prompt, system_prompt).await,
    }
}

/// 带工具调用循环的文本生成。`registry` 应已按 Agent 白名单收窄（见
/// `tools::ToolRegistry::filtered`）。当满足以下任一条件时退化为无工具单轮
/// （直接委托 [`run_agent_text`]）：Agent 未绑定 LLM / 绑定 Claude CLI / 带图片输入 /
/// 工具集为空 / api_spec 不支持工具（仅 openai、anthropic 支持）。
///
/// 退化策略：工具循环中途任何错误（含端点不支持 tools 字段）都会回退到无工具单轮，
/// 保证「带工具反而答不出」不会比原来更糟。
pub async fn run_agent_text_with_tools(
    db: &crate::db::Db,
    agent: &Agent,
    prompt: &str,
    system_prompt: Option<&str>,
    image_paths: &[PathBuf],
    registry: &ToolRegistry,
) -> Result<String> {
    // 无工具 / 带图片 / 无自定义 LLM → 老路径，保持既有行为。
    if registry.is_empty() || !image_paths.is_empty() || agent.llm_id.is_none() {
        return run_agent_text(db, agent, prompt, system_prompt, image_paths).await;
    }

    let llm_id = agent.llm_id.as_ref().unwrap();
    let cfg = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(llm_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow!("LLM 配置不存在: {}", llm_id))?;
    if !cfg.enabled {
        return Err(anyhow!("LLM 配置已禁用: {}", cfg.name));
    }

    let spec = cfg.api_spec.to_ascii_lowercase();
    // 仅 openai、anthropic 两种 wire 格式；未知按 openai 处理，与 run_agent_text 一致。
    let loop_result = if spec == "anthropic" {
        run_anthropic_tool_loop(&cfg, prompt, system_prompt, registry).await
    } else {
        run_openai_tool_loop(&cfg, prompt, system_prompt, registry).await
    };

    match loop_result {
        Ok(text) => Ok(text),
        Err(e) => {
            // 端点可能不支持 tools 字段等 → 优雅降级为无工具单轮。
            eprintln!(
                "[tools] 工具循环失败，回退无工具单轮（agent={}, spec={}): {}",
                agent.name, spec, e
            );
            run_agent_text(db, agent, prompt, system_prompt, image_paths).await
        }
    }
}

pub async fn run_system_role_text(
    db: &crate::db::Db,
    system_kind: &str,
    prompt: &str,
    fallback_system_prompt: Option<&str>,
    project_id: Option<&str>,
    recall_query: Option<&str>,
) -> Result<String> {
    let agent = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(system_kind, '') || ',') LIKE ?
           AND enabled=1
         ORDER BY created_at
         LIMIT 1",
    )
    .bind(format!("%,{},%", system_kind))
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("未配置系统角色 Agent: {}", system_kind))?;

    let Some(llm_id) = &agent.llm_id else {
        return Err(anyhow!(
            "系统角色 Agent「{}」未绑定 LLM，请在角色指派/Agent 配置中选择可用 LLM",
            agent.name
        ));
    };

    // 校验绑定的 LLM 配置存在即可（api_spec 仅 openai/anthropic，无需额外规范校验）。
    sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs WHERE id=?")
        .bind(llm_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow!("LLM 配置不存在: {}", llm_id))?;

    // 缺省召回键回退到 prompt；交给共享组装器处理 prompt_mode 组合 + 记忆召回。
    let key = recall_query.filter(|q| !q.trim().is_empty()).or(Some(prompt));
    let system_prompt = build_role_system_prompt(
        &agent,
        Some(system_kind),
        fallback_system_prompt,
        project_id,
        key,
    )
    .await;
    run_agent_text(db, &agent, prompt, system_prompt.as_deref(), &[]).await
}

/// 组装系统角色 Agent 的最终系统提示词：注册表内置（按 `prompt_mode`）+ 可选 Innate 召回
/// （受 `agent.memory_enabled` 与 `project_id` 门控；`recall_key` 为空则不召回）。
/// 供 `run_system_role_text` 与群聊编排（planner/summarizer/doc_writer/context_compressor）共用，
/// 确保两条路径的角色都用同一份升级后的内置提示词，并都能长记忆。best-effort，召回失败不影响主流程。
pub async fn build_role_system_prompt(
    agent: &Agent,
    kind: Option<&str>,
    fallback: Option<&str>,
    project_id: Option<&str>,
    recall_key: Option<&str>,
) -> Option<String> {
    let composed =
        crate::agents::roles::compose_system_prompt(kind, &agent.prompt_mode, &agent.system_prompt);
    let mut system_prompt = if composed.trim().is_empty() {
        fallback.map(|s| s.to_string())
    } else {
        Some(composed)
    };

    if agent.memory_enabled {
        if let (Some(pid), Some(k)) = (
            project_id.filter(|p| !p.is_empty()),
            recall_key.filter(|q| !q.trim().is_empty()),
        ) {
            let recalled = crate::knowledge::kb_recall(pid, k).await;
            if !recalled.trim().is_empty() {
                let head = system_prompt.take().unwrap_or_default();
                system_prompt = Some(if head.trim().is_empty() {
                    format!("## 历史经验与技能（Innate 召回）\n{}", recalled)
                } else {
                    format!("{}\n\n## 历史经验与技能（Innate 召回）\n{}", head, recalled)
                });
            }
        }
    }
    system_prompt
}

async fn run_openai_compatible(
    cfg: &LlmConfig,
    prompt: &str,
    system_prompt: Option<&str>,
) -> Result<String> {
    let client = http_client()?;
    let mut messages = Vec::new();
    if let Some(system) = system_prompt.filter(|s| !s.trim().is_empty()) {
        messages.push(serde_json::json!({ "role": "system", "content": system }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": prompt }));

    let mut req = client
        .post(join_endpoint(&cfg.endpoint, "/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": cfg.model,
            "messages": messages,
            "temperature": cfg.temperature
        }));
    if !cfg.api_key.trim().is_empty() {
        req = req.bearer_auth(&cfg.api_key);
    }

    let body = send_json(req).await?;
    body.pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("OpenAI-compatible 响应缺少 choices[0].message.content"))
}

async fn run_anthropic(
    cfg: &LlmConfig,
    prompt: &str,
    system_prompt: Option<&str>,
) -> Result<String> {
    let client = http_client()?;
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 4096,
        "temperature": cfg.temperature
    });
    if let Some(system) = system_prompt.filter(|s| !s.trim().is_empty()) {
        body["system"] = Value::String(system.to_string());
    }

    let value = send_json(
        client
            .post(join_endpoint(&cfg.endpoint, "/v1/messages"))
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body),
    )
    .await?;

    value
        .get("content")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("Anthropic 响应缺少 content[].text"))
}

/// OpenAI 兼容工具调用循环：声明 tools → 解析 message.tool_calls → 执行 → 以
/// role=tool 消息回灌 → 续轮，直到模型不再调用工具或达到轮数上限。
async fn run_openai_tool_loop(
    cfg: &LlmConfig,
    prompt: &str,
    system_prompt: Option<&str>,
    registry: &ToolRegistry,
) -> Result<String> {
    let client = http_client()?;
    let tools = openai_tools(registry);
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = system_prompt.filter(|s| !s.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));

    for _ in 0..MAX_TOOL_ITERS {
        let mut req = client
            .post(join_endpoint(&cfg.endpoint, "/v1/chat/completions"))
            .json(&json!({
                "model": cfg.model,
                "messages": messages,
                "temperature": cfg.temperature,
                "tools": tools,
                "tool_choice": "auto"
            }));
        if !cfg.api_key.trim().is_empty() {
            req = req.bearer_auth(&cfg.api_key);
        }

        let body = send_json(req).await?;
        let msg = body
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| anyhow!("OpenAI-compatible 响应缺少 choices[0].message"))?;

        let tool_calls = msg
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if tool_calls.is_empty() {
            return msg
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("OpenAI-compatible 响应缺少最终 content"));
        }

        // 助手回合（含 tool_calls）必须原样回灌，否则下一轮无法对应工具结果。
        messages.push(msg.clone());
        for call in &tool_calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let fname = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args_str = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let args = serde_json::from_str::<Value>(args_str).unwrap_or_else(|_| json!({}));
            let outcome = registry.invoke(fname, args).await;
            if !outcome.ok {
                eprintln!("[tools] 工具 `{}` 返回失败/被拦截：{}", fname, outcome.content);
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": outcome.content
            }));
        }
    }
    Err(anyhow!("工具调用超过最大轮数 {}", MAX_TOOL_ITERS))
}

/// Anthropic 工具调用循环：声明 tools → 解析 content[].type=tool_use → 执行 →
/// 以 user/tool_result 块回灌 → 续轮，直到 stop_reason≠tool_use 或达到轮数上限。
async fn run_anthropic_tool_loop(
    cfg: &LlmConfig,
    prompt: &str,
    system_prompt: Option<&str>,
    registry: &ToolRegistry,
) -> Result<String> {
    let client = http_client()?;
    let tools = anthropic_tools(registry);
    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": prompt })];

    for _ in 0..MAX_TOOL_ITERS {
        let mut body = json!({
            "model": cfg.model,
            "messages": messages,
            "max_tokens": 4096,
            "temperature": cfg.temperature,
            "tools": tools
        });
        if let Some(system) = system_prompt.filter(|s| !s.trim().is_empty()) {
            body["system"] = Value::String(system.to_string());
        }

        let value = send_json(
            client
                .post(join_endpoint(&cfg.endpoint, "/v1/messages"))
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body),
        )
        .await?;

        let content = value
            .get("content")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let stop = value.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("");
        let has_tool_use = content
            .iter()
            .any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_use"));

        if stop != "tool_use" || !has_tool_use {
            let text = content
                .iter()
                .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            return Some(text)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow!("Anthropic 响应缺少最终 text"));
        }

        // 助手回合（含 tool_use 块）原样回灌。
        messages.push(json!({ "role": "assistant", "content": content.clone() }));
        let mut results = Vec::new();
        for block in content
            .iter()
            .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_use"))
        {
            let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
            let outcome = registry.invoke(name, input).await;
            if !outcome.ok {
                eprintln!("[tools] 工具 `{}` 返回失败/被拦截：{}", name, outcome.content);
            }
            results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": outcome.content
            }));
        }
        messages.push(json!({ "role": "user", "content": results }));
    }
    Err(anyhow!("工具调用超过最大轮数 {}", MAX_TOOL_ITERS))
}

fn openai_tools(registry: &ToolRegistry) -> Vec<Value> {
    registry
        .specs()
        .into_iter()
        .map(|s| {
            json!({
                "type": "function",
                "function": {
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.parameters
                }
            })
        })
        .collect()
}

fn anthropic_tools(registry: &ToolRegistry) -> Vec<Value> {
    registry
        .specs()
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "input_schema": s.parameters
            })
        })
        .collect()
}

async fn send_json(req: reqwest::RequestBuilder) -> Result<Value> {
    let response = req.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "LLM HTTP {}: {}",
            status_code(status),
            trim_body(&text)
        ));
    }
    serde_json::from_str::<Value>(&text)
        .map_err(|e| anyhow!("LLM 响应不是有效 JSON: {}; body={}", e, trim_body(&text)))
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| anyhow!(e))
}

fn join_endpoint(endpoint: &str, path: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    if base.ends_with(path) {
        return base.to_string();
    }
    if base.ends_with("/v1") && path.starts_with("/v1/") {
        return format!("{}{}", base, &path[3..]);
    }
    format!("{}{}", base, path)
}

fn trim_body(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() > 600 {
        trimmed.chars().take(600).collect::<String>()
    } else {
        trimmed.to_string()
    }
}

fn status_code(status: StatusCode) -> String {
    status
        .canonical_reason()
        .map(|reason| format!("{} {}", status.as_u16(), reason))
        .unwrap_or_else(|| status.as_u16().to_string())
}
