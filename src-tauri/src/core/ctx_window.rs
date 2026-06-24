//! 上下文窗口（context window）推断 —— 纯 Rust，不依赖 Tauri 类型。
//!
//! 背景：官方 OpenAI / Anthropic 的 `/v1/models` 端点都**不返回**上下文长度，
//! 因此无法靠「一个标准接口」可靠拿到窗口大小。这里走两条路：
//!  1. 内置「模型名 → 窗口」查表（按子串匹配，覆盖常见模型），瞬时、无网络。
//!  2. 尽力从 `/v1/models` 探测 `context_length` / `max_model_len` 等字段
//!     （OpenRouter、vLLM、部分本地部署会返回），命中则覆盖查表值，反映真实部署。
//!
//! 这个字段是展示用的参考值，拿不到时回退「未知」，不参与任何压缩/预算逻辑。

use std::time::Duration;

/// 内置模型窗口表（token 数）。按 model 名小写子串匹配，靠前者优先。
/// 只收常见模型；未命中交给探测或回退「未知」。
const KNOWN: &[(&str, u64)] = &[
    // Anthropic Claude
    ("claude-3-5", 200_000),
    ("claude-3-7", 200_000),
    ("claude-sonnet-4", 200_000),
    ("claude-opus-4", 200_000),
    ("claude-haiku-4", 200_000),
    ("claude-3", 200_000),
    ("claude", 200_000),
    // OpenAI
    ("gpt-4.1", 1_047_576),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("gpt-4-32k", 32_768),
    ("gpt-4", 8_192),
    ("o4", 200_000),
    ("o3", 200_000),
    ("o1", 200_000),
    ("gpt-3.5-turbo", 16_385),
    // Google Gemini
    ("gemini-1.5-pro", 2_097_152),
    ("gemini-1.5", 1_048_576),
    ("gemini-2", 1_048_576),
    ("gemini", 1_048_576),
    // DeepSeek
    ("deepseek-r1", 65_536),
    ("deepseek", 65_536),
    // 阿里 Qwen
    ("qwen-max", 32_768),
    ("qwen-long", 10_000_000),
    ("qwen2.5", 131_072),
    ("qwen", 131_072),
    // Moonshot Kimi
    ("moonshot-v1-128k", 131_072),
    ("moonshot-v1-32k", 32_768),
    ("moonshot", 131_072),
    ("kimi", 131_072),
    // 智谱 GLM
    ("glm-4", 131_072),
    // Mistral
    ("mistral-large", 131_072),
    ("mixtral", 32_768),
    ("mistral", 32_768),
    // Meta Llama
    ("llama-3.1", 131_072),
    ("llama-3", 8_192),
    ("llama", 8_192),
];

/// 查内置表（按子串匹配）。
pub fn known_window(model: &str) -> Option<u64> {
    let m = model.trim().to_ascii_lowercase();
    KNOWN.iter().find(|(k, _)| m.contains(k)).map(|(_, v)| *v)
}

/// 把 token 数格式化为人类可读窗口串：1_000_000→"1M"、200_000→"200K"。
pub fn fmt_window(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// 拿不到窗口时的占位串。
pub const UNKNOWN: &str = "未知";

/// 尽力从提供方的模型列表接口探测目标模型的上下文长度（token 数）。
/// 仅当提供方在 `/v1/models` 返回 `context_length` / `max_model_len` /
/// `max_context_length` 等字段时命中；官方 OpenAI/Anthropic 不返回 → None。
async fn probe_window(endpoint: &str, api_spec: &str, model: &str, api_key: &str) -> Option<u64> {
    let base = endpoint.trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let url = if base.ends_with("/v1") {
        format!("{}/models", base)
    } else {
        format!("{}/v1/models", base)
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let mut req = client.get(&url);
    // 鉴权头按 wire 规范带上；缺 key 也照常尝试（部分本地部署免鉴权）。
    if !api_key.is_empty() {
        if api_spec.eq_ignore_ascii_case("anthropic") {
            req = req
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.bearer_auth(api_key);
        }
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;

    // 兼容多种形状：{data:[{id, context_length|max_model_len|...}]} 或顶层数组。
    let arr = json
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| json.as_array())?;
    let m = model.trim().to_ascii_lowercase();
    // 先找 id 精确/包含匹配的条目，再回退第一条。
    let entry = arr
        .iter()
        .find(|e| {
            e.get("id")
                .and_then(|v| v.as_str())
                .map(|id| id.to_ascii_lowercase() == m || id.to_ascii_lowercase().contains(&m))
                .unwrap_or(false)
        })
        .or_else(|| arr.first())?;

    for key in [
        "context_length",
        "max_model_len",
        "max_context_length",
        "context_window",
        "max_context_window_tokens",
    ] {
        if let Some(n) = entry.get(key).and_then(num_field) {
            if n > 0 {
                return Some(n);
            }
        }
    }
    // OpenRouter 把上限放在 top_provider.context_length。
    if let Some(n) = entry
        .get("top_provider")
        .and_then(|p| p.get("context_length"))
        .and_then(num_field)
    {
        if n > 0 {
            return Some(n);
        }
    }
    None
}

/// 从 JSON 数字/字符串字段中取出 u64。
fn num_field(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
}

/// 综合推断上下文窗口展示串：先尝试接口探测（最贴近真实部署），
/// 未命中再查内置表，仍无则回退「未知」。
pub async fn infer_window(endpoint: &str, api_spec: &str, model: &str, api_key: &str) -> String {
    if let Some(tokens) = probe_window(endpoint, api_spec, model, api_key).await {
        return fmt_window(tokens);
    }
    if let Some(tokens) = known_window(model) {
        return fmt_window(tokens);
    }
    UNKNOWN.to_string()
}
