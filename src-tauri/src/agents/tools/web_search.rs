//! Web 搜索工具——工具循环的第一个内置工具，用来打通「声明→调用→结果回灌」闭环。
//!
//! 纯 Rust：配置在构造时从 `app_settings` 解析好（[`WebSearchConfig::load`]），
//! Tool 本身无状态、不碰 db。结果是不可信外部输入，由 [`super::ToolRegistry::invoke`]
//! 统一过 `has_obvious_injection` + 截断，本文件不重复施加。
//!
//! 支持三种 provider（均只读）：
//! - `duckduckgo`：**默认、免 Key、原生开箱即用**。抓 lite.duckduckgo.com（失败回退 html 端点），
//!   纯 Rust 正则解析结果，无需任何配置。
//! - `searxng`   ：GET  <endpoint>/search?format=json ，自托管、无需 key。
//! - `tavily`    ：POST https://api.tavily.com/search ，需 api_key，返回带摘要的结果。
//!
//! 任何 provider 未配置/配置不全时一律退回 `duckduckgo`，保证「不配置也能搜」。
//! 可选「搜索后自动读正文」：对前几条结果调用 [`super::web_fetch::fetch_readable`]
//! 抓取正文摘录一并回灌，让 Agent 一步拿到内容而非仅链接。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::time::Duration;

use super::web_fetch::{clean_inline, fetch_readable, percent_decode};
use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;
use std::sync::Arc;

const SETTINGS_PROVIDER: &str = "web_search.provider";
const SETTINGS_API_KEY: &str = "web_search.api_key";
const SETTINGS_ENDPOINT: &str = "web_search.endpoint";
const SETTINGS_MAX_RESULTS: &str = "web_search.max_results";
const SETTINGS_FETCH_CONTENT: &str = "web_search.fetch_content";

/// 类浏览器 UA：DuckDuckGo 对空/脚本 UA 会返回空页或拦截。
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
/// 「搜索后自动读正文」时抓取的结果条数与每条正文字符上限（控制延迟与 token）。
const AUTO_READ_TOP: usize = 3;
const AUTO_READ_CHARS: usize = 1500;

/// 已解析的 Web 搜索配置。provider 缺失/不全将退回 DuckDuckGo。
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    pub provider: String,
    pub api_key: String,
    pub endpoint: String,
    pub max_results: u32,
    /// 是否默认在搜索后自动抓取前几条结果的正文摘录。
    pub fetch_content: bool,
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
        let fetch_content = get_setting(db, SETTINGS_FETCH_CONTENT)
            .await
            .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
            .unwrap_or(false);
        Self {
            provider,
            api_key,
            endpoint,
            max_results,
            fetch_content,
        }
    }

    /// 实际生效的 provider：所选 provider 配置齐全则用它，否则退回免 Key 的 DuckDuckGo。
    pub fn effective_provider(&self) -> &str {
        match self.provider.as_str() {
            "tavily" if !self.api_key.trim().is_empty() => "tavily",
            "searxng" if !self.endpoint.trim().is_empty() => "searxng",
            _ => "duckduckgo",
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

/// web_search 工厂：免配置开箱即用（默认 DuckDuckGo），任何场景都可装配。
pub struct WebSearchFactory;

#[async_trait]
impl BuiltinTool for WebSearchFactory {
    fn info(&self) -> ToolInfo {
        ToolInfo { name: "web_search", label: "联网搜索", needs_project: false }
    }
    async fn build(&self, db: &Db, _ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        // 始终可装配：未配置任何 provider 时走免 Key 的 DuckDuckGo。
        Some(Arc::new(WebSearchTool::new(WebSearchConfig::load(db).await)) as Arc<dyn Tool>)
    }
}

/// 一条搜索命中。
struct Hit {
    title: String,
    url: String,
    snippet: String,
}

/// DuckDuckGo 端点 HTML 解析函数：`(html, limit) -> 命中列表`。
type DdgParser = fn(&str, u32) -> Vec<Hit>;

/// 内置 Web 搜索工具。
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
            "联网搜索：根据查询词检索互联网，返回标题、链接与摘要。需要最新/外部事实、文档或新闻时使用。\
             可选 read_content=true 让其在搜索后自动抓取前几条结果的正文摘录一并返回；\
             若只拿到链接想读全文，再用 web_fetch 抓取具体 URL。",
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
                    },
                    "read_content": {
                        "type": "boolean",
                        "description": "为 true 时自动抓取前几条结果的正文摘录一并返回（更慢但更省往返）。"
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
        let read_content = args
            .get("read_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.cfg.fetch_content);

        let (prefix, hits) = match self.cfg.effective_provider() {
            "tavily" => self.search_tavily(query, limit).await?,
            "searxng" => self.search_searxng(query, limit).await?,
            _ => self.search_duckduckgo(query, limit).await?,
        };

        let mut out = String::new();
        if !prefix.trim().is_empty() {
            out.push_str(&format!("摘要：{}\n\n", prefix.trim()));
        }
        for (i, h) in hits.iter().enumerate() {
            out.push_str(&format!("{}. {}\n{}\n{}\n\n", i + 1, h.title, h.url, h.snippet));
        }

        if read_content {
            let bodies = self.read_top_bodies(&hits).await;
            if !bodies.trim().is_empty() {
                out.push_str("---\n以下为前几条结果的正文摘录：\n\n");
                out.push_str(&bodies);
            }
        }

        finalize(out, query)
    }
}

impl WebSearchTool {
    /// 对前若干条 http(s) 结果抓取正文摘录；单条失败仅记一行，不影响其余。
    async fn read_top_bodies(&self, hits: &[Hit]) -> String {
        let mut out = String::new();
        for h in hits
            .iter()
            .filter(|h| h.url.starts_with("http"))
            .take(AUTO_READ_TOP)
        {
            match fetch_readable(&h.url, AUTO_READ_CHARS).await {
                Ok(body) => out.push_str(&format!("【{}】{}\n{}\n\n", h.title, h.url, body)),
                Err(e) => out.push_str(&format!("【{}】{} — 正文抓取失败：{}\n\n", h.title, h.url, e)),
            }
        }
        out
    }

    async fn search_tavily(&self, query: &str, limit: u32) -> Result<(String, Vec<Hit>)> {
        let body = json!({
            "api_key": self.cfg.api_key,
            "query": query,
            "max_results": limit,
            "search_depth": "basic"
        });
        let value = post_json("https://api.tavily.com/search", body).await?;
        let answer = value
            .get("answer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let hits = value
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|r| Hit {
                        title: r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)").to_string(),
                        url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        snippet: r.get("content").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((answer, hits))
    }

    async fn search_searxng(&self, query: &str, limit: u32) -> Result<(String, Vec<Hit>)> {
        let base = self.cfg.endpoint.trim_end_matches('/');
        let url = format!("{}/search?q={}&format=json", base, urlencode(query));
        let value = get_json(&url).await?;
        let hits = value
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .take(limit as usize)
                    .map(|r| Hit {
                        title: r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)").to_string(),
                        url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        snippet: r.get("content").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((String::new(), hits))
    }

    /// DuckDuckGo（免 Key）：优先 lite 端点，解析为空时回退 html 端点。
    /// 命中反爬挑战页时返回明确错误，提示改配 SearXNG/Tavily，而非静默「无结果」。
    async fn search_duckduckgo(&self, query: &str, limit: u32) -> Result<(String, Vec<Hit>)> {
        let client = search_client()?;
        let q = urlencode(query);
        let endpoints: [(String, DdgParser); 2] = [
            (format!("https://lite.duckduckgo.com/lite/?q={q}"), parse_ddg_lite),
            (format!("https://html.duckduckgo.com/html/?q={q}"), parse_ddg_html),
        ];
        let mut challenged = false;
        for (url, parse) in endpoints {
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(body) = resp_text(resp).await {
                    let hits = parse(&body, limit);
                    if !hits.is_empty() {
                        return Ok((String::new(), hits));
                    }
                    challenged |= is_ddg_challenge(&body);
                }
            }
        }
        if challenged {
            return Err(anyhow!(
                "DuckDuckGo 暂时拦截了自动检索（反爬挑战）。请稍后重试；若频繁出现，可在「设置 → 工具 & MCP」改用 SearXNG（自托管）或 Tavily（需 Key）作为搜索源。"
            ));
        }
        Ok((String::new(), Vec::new()))
    }
}

/// 识别 DuckDuckGo 的反爬挑战/异常页（无搜索结果、仅含挑战 UI）。
fn is_ddg_challenge(body: &str) -> bool {
    body.contains("anomaly-modal") || body.contains("anomaly.js")
}

// ───────────────────────── DuckDuckGo 解析 ─────────────────────────

// lite 端点：<a ... href="..." class="result-link">标题</a> + <td class="result-snippet">摘要</td>
static RE_LITE_LINK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<a\s+[^>]*?href=["']([^"']+)["'][^>]*?class=['"]result-link['"][^>]*?>(.*?)</a>"#).unwrap()
});
static RE_LITE_SNIPPET: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)<td\s+class=['"]result-snippet['"][^>]*>(.*?)</td>"#).unwrap());

// html 端点：<a class="result__a" href="...">标题</a> + class="result__snippet"
static RE_HTML_LINK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*?class=["'][^"']*result__a[^"']*["'][^>]*?href=["']([^"']+)["'][^>]*?>(.*?)</a>"#).unwrap()
});
static RE_HTML_SNIPPET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)class=["'][^"']*result__snippet[^"']*["'][^>]*>(.*?)</(?:a|div|span)>"#).unwrap()
});

fn parse_ddg_lite(html: &str, limit: u32) -> Vec<Hit> {
    let snippets: Vec<String> = RE_LITE_SNIPPET
        .captures_iter(html)
        .map(|c| clean_inline(&c[1]))
        .collect();
    RE_LITE_LINK
        .captures_iter(html)
        .take(limit as usize)
        .enumerate()
        .map(|(i, c)| Hit {
            title: clean_inline(&c[2]),
            url: resolve_ddg_url(&c[1]),
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        })
        .filter(|h| !h.url.is_empty())
        .collect()
}

fn parse_ddg_html(html: &str, limit: u32) -> Vec<Hit> {
    let snippets: Vec<String> = RE_HTML_SNIPPET
        .captures_iter(html)
        .map(|c| clean_inline(&c[1]))
        .collect();
    RE_HTML_LINK
        .captures_iter(html)
        .take(limit as usize)
        .enumerate()
        .map(|(i, c)| Hit {
            title: clean_inline(&c[2]),
            url: resolve_ddg_url(&c[1]),
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        })
        .filter(|h| !h.url.is_empty())
        .collect()
}

/// 还原 DuckDuckGo 结果真实 URL：href 可能是 //duckduckgo.com/l/?uddg=<编码目标>&… 的跳转链。
fn resolve_ddg_url(href: &str) -> String {
    let h = href.replace("&amp;", "&");
    if let Some(idx) = h.find("uddg=") {
        let rest = &h[idx + "uddg=".len()..];
        let enc = rest.split('&').next().unwrap_or(rest);
        let decoded = percent_decode(enc);
        if !decoded.is_empty() {
            return decoded;
        }
    }
    if let Some(stripped) = h.strip_prefix("//") {
        format!("https://{stripped}")
    } else {
        h
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

/// 抓 DuckDuckGo 用的客户端：带浏览器 UA。
fn search_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(UA)
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

async fn resp_text(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("搜索服务 HTTP {}", status.as_u16()));
    }
    Ok(text)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_provider_falls_back_to_ddg() {
        let mk = |provider: &str, key: &str, ep: &str| WebSearchConfig {
            provider: provider.into(),
            api_key: key.into(),
            endpoint: ep.into(),
            max_results: 5,
            fetch_content: false,
        };
        assert_eq!(mk("", "", "").effective_provider(), "duckduckgo");
        assert_eq!(mk("tavily", "", "").effective_provider(), "duckduckgo");
        assert_eq!(mk("tavily", "k", "").effective_provider(), "tavily");
        assert_eq!(mk("searxng", "", "").effective_provider(), "duckduckgo");
        assert_eq!(mk("searxng", "", "https://s.io").effective_provider(), "searxng");
        assert_eq!(mk("duckduckgo", "", "").effective_provider(), "duckduckgo");
    }

    #[test]
    fn resolve_ddg_url_decodes_uddg() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdoc&amp;rut=abc";
        assert_eq!(resolve_ddg_url(href), "https://example.com/doc");
        assert_eq!(resolve_ddg_url("https://direct.com/x"), "https://direct.com/x");
        assert_eq!(resolve_ddg_url("//cdn.com/a"), "https://cdn.com/a");
    }

    #[test]
    fn parse_ddg_lite_extracts_hits() {
        let html = r#"
            <a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.com" class='result-link'>Title A</a>
            <td class='result-snippet'>Snippet&nbsp;A</td>
            <a rel="nofollow" href="https://b.com/page" class='result-link'>Title&amp;B</a>
            <td class='result-snippet'>Snippet B</td>
        "#;
        let hits = parse_ddg_lite(html, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://a.com");
        assert_eq!(hits[0].title, "Title A");
        assert_eq!(hits[0].snippet, "Snippet A");
        assert_eq!(hits[1].url, "https://b.com/page");
        assert_eq!(hits[1].title, "Title&B");
    }

    #[test]
    fn parse_ddg_html_extracts_hits() {
        let html = r##"
            <a class="result__a" href="https://x.com/1">First</a>
            <a class="result__snippet" href="#">snippet one</a>
            <a class="result__a" href="https://y.com/2">Second</a>
            <a class="result__snippet" href="#">snippet two</a>
        "##;
        let hits = parse_ddg_html(html, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://x.com/1");
        assert_eq!(hits[1].title, "Second");
    }
}
