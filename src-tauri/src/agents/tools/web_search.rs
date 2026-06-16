//! Web 搜索工具——工具循环的第一个内置工具，用来打通「声明→调用→结果回灌」闭环。
//!
//! 纯 Rust：配置在构造时从 `app_settings` 解析好（[`WebSearchConfig::load`]），
//! Tool 本身无状态、不碰 db。结果是不可信外部输入，由 [`super::ToolRegistry::invoke`]
//! 统一过 `has_obvious_injection` + 截断，本文件不重复施加。
//!
//! 支持两种 provider（MVP，均只读）：
//! - `tavily`  ：POST https://api.tavily.com/search ，需 api_key，返回带摘要的结果。
//! - `searxng` ：GET  <endpoint>/search?format=json ，自托管、无需 key。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;
use std::sync::Arc;

const SETTINGS_PROVIDER: &str = "web_search.provider";
const SETTINGS_API_KEY: &str = "web_search.api_key";
const SETTINGS_ENDPOINT: &str = "web_search.endpoint";
const SETTINGS_MAX_RESULTS: &str = "web_search.max_results";

/// 已解析的 Web 搜索配置。`provider` 为空表示未启用。
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    pub provider: String,
    pub api_key: String,
    pub endpoint: String,
    pub max_results: u32,
}

impl WebSearchConfig {
    /// 从 app_settings 读取配置；任意键缺失走默认值。
    pub async fn load(db: &Db) -> Self {
        let provider = get_setting(db, SETTINGS_PROVIDER)
            .await
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let api_key = crate::core::secrets::decrypt(
            &get_setting(db, SETTINGS_API_KEY).await.unwrap_or_default(),
        )
        .unwrap_or_default();
        let endpoint = get_setting(db, SETTINGS_ENDPOINT).await.unwrap_or_default();
        let max_results = get_setting(db, SETTINGS_MAX_RESULTS)
            .await
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(5)
            .clamp(1, 10);
        Self {
            provider,
            api_key,
            endpoint,
            max_results,
        }
    }

    /// 配置是否足以真正发起搜索。
    pub fn is_enabled(&self) -> bool {
        match self.provider.as_str() {
            "tavily" => !self.api_key.trim().is_empty(),
            "searxng" => !self.endpoint.trim().is_empty(),
            _ => false,
        }
    }
}

async fn get_setting(db: &Db, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// web_search 工厂：不依赖项目上下文，任何场景下只要配置了 Provider 即可装配。
pub struct WebSearchFactory;

#[async_trait]
impl BuiltinTool for WebSearchFactory {
    fn info(&self) -> ToolInfo {
        ToolInfo { name: "web_search", label: "联网搜索", needs_project: false }
    }
    async fn build(&self, db: &Db, _ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        let cfg = WebSearchConfig::load(db).await;
        cfg.is_enabled()
            .then(|| Arc::new(WebSearchTool::new(cfg)) as Arc<dyn Tool>)
    }
}

/// 内置 Web 搜索工具。仅在 [`WebSearchConfig::is_enabled`] 时注册进注册表。
pub struct WebSearchTool {
    cfg: WebSearchConfig,
}

impl WebSearchTool {
    pub fn new(cfg: WebSearchConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_search",
            "联网搜索：根据查询词检索互联网，返回标题、链接与摘要。需要最新/外部事实、文档或新闻时使用。",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索查询词，使用自然语言或关键词。"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "返回结果条数（1-10），默认按系统配置。",
                        "minimum": 1,
                        "maximum": 10
                    }
                },
                "required": ["query"]
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("缺少参数 query"))?;
        let limit = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as u32).clamp(1, 10))
            .unwrap_or(self.cfg.max_results);

        match self.cfg.provider.as_str() {
            "tavily" => self.search_tavily(query, limit).await,
            "searxng" => self.search_searxng(query, limit).await,
            other => Err(anyhow!(
                "Web 搜索未配置或 provider 不支持：{}（请在设置中配置 tavily 或 searxng）",
                if other.is_empty() { "未设置" } else { other }
            )),
        }
    }
}

impl WebSearchTool {
    async fn search_tavily(&self, query: &str, limit: u32) -> Result<String> {
        let body = json!({
            "api_key": self.cfg.api_key,
            "query": query,
            "max_results": limit,
            "search_depth": "basic"
        });
        let value = post_json("https://api.tavily.com/search", body).await?;
        let results = value
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = String::new();
        if let Some(ans) = value.get("answer").and_then(|v| v.as_str()) {
            if !ans.trim().is_empty() {
                out.push_str(&format!("摘要：{}\n\n", ans.trim()));
            }
        }
        for (i, r) in results.iter().enumerate() {
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)");
            let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let content = r.get("content").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("{}. {}\n{}\n{}\n\n", i + 1, title, url, content.trim()));
        }
        finalize(out, query)
    }

    async fn search_searxng(&self, query: &str, limit: u32) -> Result<String> {
        let base = self.cfg.endpoint.trim_end_matches('/');
        let url = format!(
            "{}/search?q={}&format=json",
            base,
            urlencode(query)
        );
        let value = get_json(&url).await?;
        let results = value
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = String::new();
        for (i, r) in results.iter().take(limit as usize).enumerate() {
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)");
            let link = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = r.get("content").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("{}. {}\n{}\n{}\n\n", i + 1, title, link, snippet.trim()));
        }
        finalize(out, query)
    }
}

fn finalize(out: String, query: &str) -> Result<String> {
    if out.trim().is_empty() {
        Ok(format!("「{}」未检索到结果。", query))
    } else {
        Ok(out.trim().to_string())
    }
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| anyhow!(e))
}

async fn post_json(url: &str, body: Value) -> Result<Value> {
    let resp = http_client()?.post(url).json(&body).send().await?;
    parse_response(resp).await
}

async fn get_json(url: &str) -> Result<Value> {
    let resp = http_client()?.get(url).send().await?;
    parse_response(resp).await
}

async fn parse_response(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(anyhow!("搜索服务 HTTP {}: {}", status.as_u16(), snippet));
    }
    serde_json::from_str::<Value>(&text).map_err(|e| anyhow!("搜索响应非 JSON：{}", e))
}

/// 极简 URL 查询编码（仅转义会破坏 query string 的字符）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
