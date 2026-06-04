use crate::models::agent::Agent;
use crate::models::llm_config::LlmConfig;
use anyhow::{anyhow, Result};
use reqwest::StatusCode;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub async fn run_agent_text(
    db: &crate::db::Db,
    agent: &Agent,
    prompt: &str,
    system_prompt: Option<&str>,
    image_paths: &[PathBuf],
) -> Result<String> {
    let Some(llm_id) = &agent.llm_id else {
        return crate::agents::local_claude::run_text_with_images(prompt, system_prompt, image_paths)
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

    let provider = cfg.provider.to_ascii_lowercase();
    if provider.contains("claude-cli") {
        return crate::agents::local_claude::run_text_with_model_and_images(
            prompt,
            system_prompt,
            image_paths,
            Some(&cfg.model),
        )
        .await;
    }

    if !image_paths.is_empty() {
        return Err(anyhow!(
            "当前 LLM 适配器暂不支持图片输入，请改用 Claude CLI 或移除图片附件"
        ));
    }

    if provider.contains("ollama") {
        run_ollama(&cfg, prompt, system_prompt).await
    } else if provider.contains("anthropic") {
        run_anthropic(&cfg, prompt, system_prompt).await
    } else {
        run_openai_compatible(&cfg, prompt, system_prompt).await
    }
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

async fn run_ollama(
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

    let value = send_json(
        client
            .post(join_endpoint(&cfg.endpoint, "/api/chat"))
            .json(&serde_json::json!({
                "model": cfg.model,
                "messages": messages,
                "stream": false,
                "options": { "temperature": cfg.temperature }
            })),
    )
    .await?;

    value
        .pointer("/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Ollama 响应缺少 message.content"))
}

async fn send_json(req: reqwest::RequestBuilder) -> Result<Value> {
    let response = req.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("LLM HTTP {}: {}", status_code(status), trim_body(&text)));
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
