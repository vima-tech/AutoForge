//! Web 搜索工具——工具循环的核心联网能力，打通「声明→调用→结果回灌」闭环。
//!
//! 纯 Rust：配置在构造时从 `app_settings` 解析好（[`WebSearchConfig::load`]），
//! Tool 本身无状态、不碰 db。结果是不可信外部输入，由 [`super::ToolRegistry::invoke`]
//! 统一过 `has_obvious_injection` + 截断，本文件不重复施加。
//!
//! ## 多源并行（multi-source）
//! 支持四种 provider（均只读）：
//! - `duckduckgo`：**默认、免 Key、原生开箱即用**。抓 lite.duckduckgo.com（失败回退 html 端点）。
//! - `searxng`   ：GET  <endpoint>/search?format=json ，自托管、无需 key（支持 time_range）。
//! - `tavily`    ：POST https://api.tavily.com/search ，需 key，advanced 深度 + answer 摘要。
//! - `brave`     ：GET  https://api.search.brave.com/res/v1/web/search ，需 key，独立索引。
//!
//! 开启 `multi_source` 时一次查询**并行 fan-out** 到所有已配置 provider，按 URL 归一去重、
//! 多源命中加权，再按与 query 的相关性重排（[`rerank`]）。任一源失败/吃反爬只跳过，其余兜底。
//! 关闭时退回单源（首选 provider，配置不全则免 Key 的 DuckDuckGo），与改造前行为一致。
//!
//! ## 检索控制
//! 工具参数支持 `time_range`（day/week/month/year）、`site`、`include_domains`、`exclude_domains`，
//! 各 provider 按原生能力下推，最终再统一做一次域名过滤兜底。
//!
//! ## 缓存
//! `(provider 集合 + 归一 query + 参数)` → 命中列表，进程内 TTL 缓存（默认 900s），
//! 同一查询在 TTL 内零网络往返。
//!
//! 可选「搜索后自动读正文」：对前几条结果调用 [`super::web_fetch::fetch_readable`]
//! 抓取正文摘录一并回灌，让 Agent 一步拿到内容而非仅链接。
//!
//! [`run_search`] 是 provider 无关的检索入口，[`deep_research`](super::deep_research) 复用它做多子查询研究。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::web_fetch::{clean_inline, fetch_readable, percent_decode};
use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;
use std::sync::Arc;

const SETTINGS_PROVIDER: &str = "web_search.provider";
const SETTINGS_API_KEY: &str = "web_search.api_key"; // 旧版 Tavily key（向后兼容）
const SETTINGS_TAVILY_KEY: &str = "web_search.tavily_key";
const SETTINGS_BRAVE_KEY: &str = "web_search.brave_key";
const SETTINGS_ENDPOINT: &str = "web_search.endpoint"; // SearXNG endpoint
const SETTINGS_MAX_RESULTS: &str = "web_search.max_results";
const SETTINGS_FETCH_CONTENT: &str = "web_search.fetch_content";
const SETTINGS_MULTI_SOURCE: &str = "web_search.multi_source";
const SETTINGS_CACHE_TTL: &str = "web_search.cache_ttl_secs";

/// 类浏览器 UA：DuckDuckGo 对空/脚本 UA 会返回空页或拦截。
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
/// 「搜索后自动读正文」时抓取的结果条数与每条正文字符上限（控制延迟与 token）。
const AUTO_READ_TOP: usize = 3;
const AUTO_READ_CHARS: usize = 1500;
/// 单个 provider 的并行检索硬超时（多源时不让最慢的源拖垮整次调用）。
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(12);

// ───────────────────────── 配置 ─────────────────────────

/// 已解析的 Web 搜索配置。各 provider 独立持 key，缺配置的源自动跳过。
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    /// 首选 provider（单源模式用；multi_source 关时生效）。
    pub primary: String,
    pub tavily_key: String,
    pub brave_key: String,
    pub searxng_endpoint: String,
    pub max_results: u32,
    /// 是否默认在搜索后自动抓取前几条结果的正文摘录。
    pub fetch_content: bool,
    /// 是否多源并行 fan-out（默认开：有几个源用几个源，DDG 永远兜底）。
    pub multi_source: bool,
    /// 结果缓存 TTL（秒，0 表示禁用缓存）。
    pub cache_ttl_secs: u64,
}

impl WebSearchConfig {
    /// 从 app_settings 读取配置；任意键缺失走默认值。
    pub async fn load(db: &Db) -> Self {
        let primary = get_setting(db, SETTINGS_PROVIDER)
            .await
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        // Tavily key：优先新键，回退旧 `web_search.api_key`（历史上只有 Tavily 用 key）。
        let tavily_key = {
            let new = decrypt_setting(db, SETTINGS_TAVILY_KEY).await;
            if new.trim().is_empty() {
                decrypt_setting(db, SETTINGS_API_KEY).await
            } else {
                new
            }
        };
        let brave_key = decrypt_setting(db, SETTINGS_BRAVE_KEY).await;
        let searxng_endpoint = get_setting(db, SETTINGS_ENDPOINT).await.unwrap_or_default();
        let max_results = get_setting(db, SETTINGS_MAX_RESULTS)
            .await
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(5)
            .clamp(1, 10);
        let fetch_content = bool_setting(db, SETTINGS_FETCH_CONTENT, false).await;
        // 默认开多源：单 DDG 时等价单源、零回归；配了 key 才真正 fan-out。
        let multi_source = bool_setting(db, SETTINGS_MULTI_SOURCE, true).await;
        let cache_ttl_secs = get_setting(db, SETTINGS_CACHE_TTL)
            .await
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(900)
            .min(3600);
        Self {
            primary,
            tavily_key,
            brave_key,
            searxng_endpoint,
            max_results,
            fetch_content,
            multi_source,
            cache_ttl_secs,
        }
    }

    /// 本次检索实际参与的 provider 列表。
    /// - multi_source：所有已配置的源（DDG 永远在内做免 Key 兜底 + 结果多样性）。
    /// - 单源：首选 provider 若已配置则用之，否则退回 DuckDuckGo。
    pub fn active_providers(&self) -> Vec<Provider> {
        let mut v: Vec<Provider> = Vec::new();
        if !self.tavily_key.trim().is_empty() {
            v.push(Provider::Tavily);
        }
        if !self.brave_key.trim().is_empty() {
            v.push(Provider::Brave);
        }
        if !self.searxng_endpoint.trim().is_empty() {
            v.push(Provider::Searxng);
        }
        v.push(Provider::Duckduckgo); // 免 Key 兜底，始终可用

        if self.multi_source {
            return v;
        }
        // 单源：按首选挑一个已配置的，否则 DDG。
        let want = match self.primary.as_str() {
            "tavily" => Provider::Tavily,
            "brave" => Provider::Brave,
            "searxng" => Provider::Searxng,
            _ => Provider::Duckduckgo,
        };
        if v.contains(&want) {
            vec![want]
        } else {
            vec![Provider::Duckduckgo]
        }
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Duckduckgo,
    Searxng,
    Tavily,
    Brave,
}

impl Provider {
    fn label(&self) -> &'static str {
        match self {
            Provider::Duckduckgo => "ddg",
            Provider::Searxng => "searxng",
            Provider::Tavily => "tavily",
            Provider::Brave => "brave",
        }
    }
    /// 源权重：用于多源重排时给更可靠的检索 API 轻微加分。
    fn weight(&self) -> f32 {
        match self {
            Provider::Tavily => 1.5,
            Provider::Brave => 1.3,
            Provider::Searxng => 1.1,
            Provider::Duckduckgo => 1.0,
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

async fn decrypt_setting(db: &Db, key: &str) -> String {
    crate::core::secrets::decrypt(&get_setting(db, key).await.unwrap_or_default()).unwrap_or_default()
}

async fn bool_setting(db: &Db, key: &str, default: bool) -> bool {
    match get_setting(db, key).await {
        Some(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"),
        None => default,
    }
}

// ───────────────────────── 检索参数 ─────────────────────────

/// 一次检索的 provider 无关参数。供 web_search 工具与 deep_research 共用。
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub query: String,
    pub limit: u32,
    /// 时间窗：day / week / month / year（各源原生能力不同，DDG 不支持则忽略）。
    pub time_range: Option<String>,
    /// 仅在这些域名内检索（如 ["docs.rs","github.com"]）。
    pub include_domains: Vec<String>,
    /// 排除这些域名。
    pub exclude_domains: Vec<String>,
    /// 限定单站点（等价 site: 语法；并会并入 include_domains 做兜底过滤）。
    pub site: Option<String>,
}

impl SearchParams {
    pub fn new(query: impl Into<String>, limit: u32) -> Self {
        Self {
            query: query.into(),
            limit: limit.clamp(1, 10),
            ..Default::default()
        }
    }

    /// 归一时间窗为各源认得的小写枚举；非法值丢弃。
    fn norm_time(&self) -> Option<&str> {
        match self.time_range.as_deref().map(str::trim) {
            Some("day") | Some("d") => Some("day"),
            Some("week") | Some("w") => Some("week"),
            Some("month") | Some("m") => Some("month"),
            Some("year") | Some("y") => Some("year"),
            _ => None,
        }
    }

    /// 合并 site 到 include 域名清单（去空白、小写）。
    fn effective_includes(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .include_domains
            .iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        if let Some(s) = self.site.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
            if !s.is_empty() && !v.contains(&s) {
                v.push(s);
            }
        }
        v
    }

    fn excludes_norm(&self) -> Vec<String> {
        self.exclude_domains
            .iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect()
    }

    /// 把 site/域名过滤拼进 query（DDG 这类无原生过滤的源用）。
    fn ddg_query(&self) -> String {
        let mut q = self.query.clone();
        let inc = self.effective_includes();
        // DDG 只能表达单 site:，多域名时取第一个（其余靠最终域名过滤兜底）。
        if let Some(first) = inc.first() {
            q.push_str(&format!(" site:{first}"));
        }
        for ex in self.excludes_norm() {
            q.push_str(&format!(" -site:{ex}"));
        }
        q
    }
}

// ───────────────────────── 命中 + 工具 ─────────────────────────

/// 一条搜索命中。
#[derive(Debug, Clone)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// 命中此结果的源（多源去重后可能多个）。
    pub sources: Vec<String>,
    /// 重排得分（仅排序用，不回灌）。
    pub score: f32,
}

impl Hit {
    fn new(title: String, url: String, snippet: String, source: &str) -> Self {
        Self {
            title,
            url,
            snippet,
            sources: vec![source.to_string()],
            score: 0.0,
        }
    }
}

/// DuckDuckGo 端点 HTML 解析函数：`(html, limit) -> 命中列表`。
type DdgParser = fn(&str, u32) -> Vec<Hit>;

/// 内置 Web 搜索工具。
pub struct WebSearchTool {
    db: Db,
    cfg: WebSearchConfig,
}

impl WebSearchTool {
    pub fn new(db: Db, cfg: WebSearchConfig) -> Self {
        Self { db, cfg }
    }
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
        Some(Arc::new(WebSearchTool::new(db.clone(), WebSearchConfig::load(db).await)) as Arc<dyn Tool>)
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_search",
            "联网搜索：根据查询词检索互联网，返回标题、链接与摘要。需要最新/外部事实、文档或新闻时使用。\
             支持多源并行检索与按时间/站点收窄；可选 read_content=true 让其在搜索后自动抓取前几条结果的正文摘录一并返回；\
             若只拿到链接想读全文，再用 web_fetch 抓取具体 URL；需要对一个复杂主题做系统性多角度调研时改用 deep_research。",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索查询词，使用自然语言或关键词。" },
                    "max_results": {
                        "type": "integer",
                        "description": "返回结果条数（1-10），默认按系统配置。",
                        "minimum": 1, "maximum": 10
                    },
                    "time_range": {
                        "type": "string",
                        "description": "只要某时间窗内的结果：day/week/month/year（找最新动态时用）。",
                        "enum": ["day", "week", "month", "year"]
                    },
                    "site": { "type": "string", "description": "限定单个站点检索，如 \"docs.rs\" 或 \"github.com\"。" },
                    "include_domains": {
                        "type": "array", "items": { "type": "string" },
                        "description": "只在这些域名内检索（可多个）。"
                    },
                    "exclude_domains": {
                        "type": "array", "items": { "type": "string" },
                        "description": "排除这些域名。"
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

        let mut params = SearchParams::new(query, limit);
        params.time_range = args.get("time_range").and_then(|v| v.as_str()).map(|s| s.to_string());
        params.site = args.get("site").and_then(|v| v.as_str()).map(|s| s.to_string());
        params.include_domains = str_array(args.get("include_domains"));
        params.exclude_domains = str_array(args.get("exclude_domains"));

        let (answer, hits) = run_search(&self.db, &self.cfg, &params).await?;

        let mut out = String::new();
        if !answer.trim().is_empty() {
            out.push_str(&format!("摘要：{}\n\n", answer.trim()));
        }
        for (i, h) in hits.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}\n{}\n{}{}\n\n",
                i + 1,
                h.title,
                h.url,
                h.snippet,
                source_tag(h),
            ));
        }

        if read_content {
            let bodies = read_top_bodies(&hits).await;
            if !bodies.trim().is_empty() {
                out.push_str("---\n以下为前几条结果的正文摘录：\n\n");
                out.push_str(&bodies);
            }
        }

        finalize(out, query)
    }
}

/// 多源结果的来源标注（单源不标，避免噪声）。
fn source_tag(h: &Hit) -> String {
    if h.sources.len() > 1 {
        format!("  [{}]", h.sources.join("+"))
    } else {
        String::new()
    }
}

fn str_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 对前若干条 http(s) 结果抓取正文摘录；单条失败仅记一行，不影响其余。
pub async fn read_top_bodies(hits: &[Hit]) -> String {
    let mut out = String::new();
    for h in hits.iter().filter(|h| h.url.starts_with("http")).take(AUTO_READ_TOP) {
        match fetch_readable(&h.url, AUTO_READ_CHARS).await {
            Ok(body) => out.push_str(&format!("【{}】{}\n{}\n\n", h.title, h.url, body)),
            Err(e) => out.push_str(&format!("【{}】{} — 正文抓取失败：{}\n\n", h.title, h.url, e)),
        }
    }
    out
}

// ───────────────────────── provider 无关检索入口 ─────────────────────────

/// 执行一次检索：缓存命中直接返回；否则按配置 fan-out 到各 provider（并行），
/// 合并去重 + 重排 + 域名过滤 + 截断，并写入缓存。返回 `(answer, hits)`，
/// `answer` 仅当 Tavily 参与时可能非空。
pub async fn run_search(
    db: &Db,
    cfg: &WebSearchConfig,
    params: &SearchParams,
) -> Result<(String, Vec<Hit>)> {
    let _ = db; // 预留：未来按项目/用户调整源；当前检索不依赖 db。
    let providers = cfg.active_providers();
    let cache_key = cache_key(&providers, params);
    if cfg.cache_ttl_secs > 0 {
        if let Some(cached) = cache_get(&cache_key, cfg.cache_ttl_secs) {
            return Ok(cached);
        }
    }

    // 各 provider 取稍多的候选，给跨源去重 + 重排留余量。
    let per_provider = params.limit.max(5).min(10);

    let mut answer = String::new();
    let mut futs = Vec::new();
    for p in &providers {
        let p = *p;
        let cfg = cfg.clone();
        let params = params.clone();
        futs.push(async move {
            let r = tokio::time::timeout(
                PROVIDER_TIMEOUT,
                search_one(&cfg, p, &params, per_provider),
            )
            .await;
            (p, r)
        });
    }
    let results = futures::future::join_all(futs).await;

    let mut merged: Vec<Hit> = Vec::new();
    for (p, r) in results {
        match r {
            Ok(Ok((ans, hits))) => {
                if answer.is_empty() && !ans.trim().is_empty() {
                    answer = ans;
                }
                merge_hits(&mut merged, hits);
            }
            Ok(Err(e)) => tracing::debug!("[web_search] provider {} 失败：{}", p.label(), e),
            Err(_) => tracing::debug!("[web_search] provider {} 超时", p.label()),
        }
    }

    // 域名过滤兜底（确保 include/exclude 在所有源上一致生效）。
    apply_domain_filter(&mut merged, params);
    rerank(&mut merged, &params.query);
    merged.truncate(params.limit as usize);

    let out = (answer, merged);
    if cfg.cache_ttl_secs > 0 {
        cache_put(cache_key, out.clone());
    }
    Ok(out)
}

/// 合并多路检索结果（如 deep_research 的多子查询）：跨池按归一 URL 去重、
/// 按 query 重排、截断到 `limit`。供 [`deep_research`](super::deep_research) 复用。
pub fn combine_hits(pools: Vec<Vec<Hit>>, query: &str, limit: usize) -> Vec<Hit> {
    let mut acc: Vec<Hit> = Vec::new();
    for pool in pools {
        merge_hits(&mut acc, pool);
    }
    rerank(&mut acc, query);
    acc.truncate(limit);
    acc
}

/// 跨源合并：URL 归一相同则合并 sources（并保留更长的标题/摘要）。
fn merge_hits(acc: &mut Vec<Hit>, incoming: Vec<Hit>) {
    for h in incoming {
        let key = normalize_url(&h.url);
        if key.is_empty() {
            continue;
        }
        if let Some(existing) = acc.iter_mut().find(|e| normalize_url(&e.url) == key) {
            for s in h.sources {
                if !existing.sources.contains(&s) {
                    existing.sources.push(s);
                }
            }
            if h.title.len() > existing.title.len() {
                existing.title = h.title;
            }
            if h.snippet.len() > existing.snippet.len() {
                existing.snippet = h.snippet;
            }
        } else {
            acc.push(h);
        }
    }
}

/// 单 provider 检索分发。
async fn search_one(
    cfg: &WebSearchConfig,
    provider: Provider,
    params: &SearchParams,
    limit: u32,
) -> Result<(String, Vec<Hit>)> {
    match provider {
        Provider::Tavily => search_tavily(cfg, params, limit).await,
        Provider::Brave => search_brave(cfg, params, limit).await,
        Provider::Searxng => search_searxng(cfg, params, limit).await,
        Provider::Duckduckgo => search_duckduckgo(params, limit).await,
    }
}

// ───────────────────────── Tavily（advanced） ─────────────────────────

async fn search_tavily(
    cfg: &WebSearchConfig,
    params: &SearchParams,
    limit: u32,
) -> Result<(String, Vec<Hit>)> {
    let mut body = json!({
        "api_key": cfg.tavily_key,
        "query": params.query,
        "max_results": limit,
        "search_depth": "advanced",
        "include_answer": true
    });
    if let Some(t) = params.norm_time() {
        body["time_range"] = json!(t);
    }
    let inc = params.effective_includes();
    if !inc.is_empty() {
        body["include_domains"] = json!(inc);
    }
    let exc = params.excludes_norm();
    if !exc.is_empty() {
        body["exclude_domains"] = json!(exc);
    }
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
                .map(|r| {
                    Hit::new(
                        r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)").to_string(),
                        r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        r.get("content").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
                        "tavily",
                    )
                })
                .filter(|h| !h.url.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok((answer, hits))
}

// ───────────────────────── Brave Search API ─────────────────────────

async fn search_brave(
    cfg: &WebSearchConfig,
    params: &SearchParams,
    limit: u32,
) -> Result<(String, Vec<Hit>)> {
    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencode(&params.query),
        limit
    );
    // Brave freshness：pd/pw/pm/py。
    if let Some(f) = params.norm_time().map(|t| match t {
        "day" => "pd",
        "week" => "pw",
        "month" => "pm",
        _ => "py",
    }) {
        url.push_str(&format!("&freshness={f}"));
    }
    let client = http_client()?;
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", cfg.brave_key.trim())
        .send()
        .await?;
    let value = parse_response(resp).await?;
    let hits = value
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    Hit::new(
                        clean_inline(r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)")),
                        r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        clean_inline(r.get("description").and_then(|v| v.as_str()).unwrap_or("")),
                        "brave",
                    )
                })
                .filter(|h| !h.url.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok((String::new(), hits))
}

// ───────────────────────── SearXNG（增强） ─────────────────────────

async fn search_searxng(
    cfg: &WebSearchConfig,
    params: &SearchParams,
    limit: u32,
) -> Result<(String, Vec<Hit>)> {
    let base = cfg.searxng_endpoint.trim_end_matches('/');
    let mut url = format!("{}/search?q={}&format=json", base, urlencode(&params.ddg_query()));
    if let Some(t) = params.norm_time() {
        url.push_str(&format!("&time_range={t}"));
    }
    let value = get_json(&url).await?;
    let hits = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(limit as usize)
                .map(|r| {
                    Hit::new(
                        r.get("title").and_then(|v| v.as_str()).unwrap_or("(无标题)").to_string(),
                        r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        r.get("content").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
                        "searxng",
                    )
                })
                .filter(|h| !h.url.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok((String::new(), hits))
}

// ───────────────────────── DuckDuckGo（免 Key 兜底） ─────────────────────────

/// DuckDuckGo（免 Key）：优先 lite 端点，解析为空时回退 html 端点。
/// 命中反爬挑战页时返回明确错误，提示改配 SearXNG/Tavily/Brave，而非静默「无结果」。
async fn search_duckduckgo(params: &SearchParams, limit: u32) -> Result<(String, Vec<Hit>)> {
    let client = search_client()?;
    let q = urlencode(&params.ddg_query());
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
            "DuckDuckGo 暂时拦截了自动检索（反爬挑战）。请稍后重试；若频繁出现，可在「设置 → 工具 & MCP」改用 SearXNG（自托管）/ Brave / Tavily 作为搜索源。"
        ));
    }
    Ok((String::new(), Vec::new()))
}

/// 识别 DuckDuckGo 的反爬挑战/异常页（无搜索结果、仅含挑战 UI）。
fn is_ddg_challenge(body: &str) -> bool {
    body.contains("anomaly-modal") || body.contains("anomaly.js")
}

// ───────────────────────── 重排 + 域名过滤 + URL 归一 ─────────────────────────

/// 轻量相关性重排（BM25-lite）：query token 命中标题计重、命中摘要计轻，
/// 叠加多源命中加权与源权重。无外部依赖、确定性，足以把最相关的结果顶到前面。
pub fn rerank(hits: &mut [Hit], query: &str) {
    let terms = tokenize(query);
    for h in hits.iter_mut() {
        let title = h.title.to_ascii_lowercase();
        let snip = h.snippet.to_ascii_lowercase();
        let mut s = 0.0f32;
        for t in &terms {
            if title.contains(t.as_str()) {
                s += 3.0;
            }
            if snip.contains(t.as_str()) {
                s += 1.0;
            }
        }
        // 多源命中：每多一个源 +0.6，并叠加最强源权重。
        let src_w = h
            .sources
            .iter()
            .map(|s| match s.as_str() {
                "tavily" => Provider::Tavily.weight(),
                "brave" => Provider::Brave.weight(),
                "searxng" => Provider::Searxng.weight(),
                _ => Provider::Duckduckgo.weight(),
            })
            .fold(0.0f32, f32::max);
        s += src_w + (h.sources.len() as f32 - 1.0) * 0.6;
        h.score = s;
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
}

fn tokenize(q: &str) -> Vec<String> {
    q.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

/// 统一域名过滤：include 非空时仅保留命中其一的，exclude 命中则剔除。
fn apply_domain_filter(hits: &mut Vec<Hit>, params: &SearchParams) {
    let inc = params.effective_includes();
    let exc = params.excludes_norm();
    if inc.is_empty() && exc.is_empty() {
        return;
    }
    hits.retain(|h| {
        let host = host_of_url(&h.url);
        if !inc.is_empty() && !inc.iter().any(|d| host_matches(&host, d)) {
            return false;
        }
        if exc.iter().any(|d| host_matches(&host, d)) {
            return false;
        }
        true
    });
}

/// 域名匹配：host == d 或 host 以 ".d" 结尾（子域命中）。
fn host_matches(host: &str, d: &str) -> bool {
    host == d || host.ends_with(&format!(".{d}"))
}

fn host_of_url(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|a| a.split(['/', '?', '#']).next())
        .map(|a| a.rsplit('@').next().unwrap_or(a))
        .map(|a| a.split(':').next().unwrap_or(a))
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// URL 归一（去 scheme、www. 前缀、末尾 /、fragment），用于跨源去重。
fn normalize_url(url: &str) -> String {
    let u = url.trim();
    let no_scheme = u.split("://").nth(1).unwrap_or(u);
    let no_frag = no_scheme.split('#').next().unwrap_or(no_scheme);
    let no_www = no_frag.strip_prefix("www.").unwrap_or(no_frag);
    no_www.trim_end_matches('/').to_ascii_lowercase()
}

// ───────────────────────── 进程内 TTL 缓存 ─────────────────────────

static CACHE: Lazy<Mutex<HashMap<String, (Instant, (String, Vec<Hit>))>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
const CACHE_CAP: usize = 256;

fn cache_key(providers: &[Provider], p: &SearchParams) -> String {
    let mut srcs: Vec<&str> = providers.iter().map(|p| p.label()).collect();
    srcs.sort_unstable();
    format!(
        "{}|q={}|n={}|t={}|site={}|inc={}|exc={}",
        srcs.join(","),
        p.query.trim().to_ascii_lowercase(),
        p.limit,
        p.norm_time().unwrap_or("-"),
        p.site.as_deref().unwrap_or("-"),
        p.effective_includes().join("+"),
        p.excludes_norm().join("+"),
    )
}

fn cache_get(key: &str, ttl: u64) -> Option<(String, Vec<Hit>)> {
    let mut g = CACHE.lock().ok()?;
    if let Some((t, v)) = g.get(key) {
        if t.elapsed() < Duration::from_secs(ttl) {
            return Some(v.clone());
        }
        g.remove(key);
    }
    None
}

fn cache_put(key: String, val: (String, Vec<Hit>)) {
    if let Ok(mut g) = CACHE.lock() {
        if g.len() >= CACHE_CAP {
            // 简单老化：清掉超过 15min 的，避免无限增长；仍满则整清。
            let now = Instant::now();
            g.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(900));
            if g.len() >= CACHE_CAP {
                g.clear();
            }
        }
        g.insert(key, (Instant::now(), val));
    }
}

// ───────────────────────── DuckDuckGo HTML 解析 ─────────────────────────

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
        .map(|(i, c)| {
            Hit::new(
                clean_inline(&c[2]),
                resolve_ddg_url(&c[1]),
                snippets.get(i).cloned().unwrap_or_default(),
                "ddg",
            )
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
        .map(|(i, c)| {
            Hit::new(
                clean_inline(&c[2]),
                resolve_ddg_url(&c[1]),
                snippets.get(i).cloned().unwrap_or_default(),
                "ddg",
            )
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

// ───────────────────────── HTTP 辅助 ─────────────────────────

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

    fn cfg(tavily: &str, brave: &str, searx: &str, multi: bool, primary: &str) -> WebSearchConfig {
        WebSearchConfig {
            primary: primary.into(),
            tavily_key: tavily.into(),
            brave_key: brave.into(),
            searxng_endpoint: searx.into(),
            max_results: 5,
            fetch_content: false,
            multi_source: multi,
            cache_ttl_secs: 0,
        }
    }

    #[test]
    fn single_source_falls_back_to_ddg() {
        // 单源模式：无配置 → DDG。
        assert_eq!(cfg("", "", "", false, "").active_providers(), vec![Provider::Duckduckgo]);
        // 选了 tavily 但没 key → 退回 DDG。
        assert_eq!(cfg("", "", "", false, "tavily").active_providers(), vec![Provider::Duckduckgo]);
        // 配了 key 且选 tavily → tavily。
        assert_eq!(cfg("k", "", "", false, "tavily").active_providers(), vec![Provider::Tavily]);
        // 选 brave 配了 key → brave。
        assert_eq!(cfg("", "bk", "", false, "brave").active_providers(), vec![Provider::Brave]);
    }

    #[test]
    fn multi_source_fans_out_to_all_configured() {
        let ps = cfg("k", "bk", "https://s.io", true, "").active_providers();
        assert!(ps.contains(&Provider::Tavily));
        assert!(ps.contains(&Provider::Brave));
        assert!(ps.contains(&Provider::Searxng));
        assert!(ps.contains(&Provider::Duckduckgo)); // 始终兜底
        assert_eq!(ps.len(), 4);
    }

    #[test]
    fn multi_source_with_no_keys_is_just_ddg() {
        assert_eq!(cfg("", "", "", true, "").active_providers(), vec![Provider::Duckduckgo]);
    }

    #[test]
    fn ddg_query_appends_site_and_excludes() {
        let mut p = SearchParams::new("rust async", 5);
        p.site = Some("docs.rs".into());
        p.exclude_domains = vec!["pinterest.com".into()];
        let q = p.ddg_query();
        assert!(q.contains("site:docs.rs"));
        assert!(q.contains("-site:pinterest.com"));
    }

    #[test]
    fn merge_dedups_by_normalized_url_and_unions_sources() {
        let mut acc = vec![Hit::new("A".into(), "https://example.com/x".into(), "s1".into(), "ddg")];
        merge_hits(
            &mut acc,
            vec![Hit::new("A longer title".into(), "https://www.example.com/x/".into(), "s2 longer".into(), "tavily")],
        );
        assert_eq!(acc.len(), 1, "归一后是同一 URL，应合并");
        assert_eq!(acc[0].sources, vec!["ddg".to_string(), "tavily".to_string()]);
        assert_eq!(acc[0].title, "A longer title", "保留更长标题");
        assert_eq!(acc[0].snippet, "s2 longer");
    }

    #[test]
    fn rerank_promotes_title_matches_and_multi_source() {
        let mut hits = vec![
            Hit::new("irrelevant page".into(), "https://a.com".into(), "nothing".into(), "ddg"),
            Hit {
                sources: vec!["tavily".into(), "brave".into()],
                ..Hit::new("rust async guide".into(), "https://b.com".into(), "rust async".into(), "tavily")
            },
        ];
        rerank(&mut hits, "rust async");
        assert_eq!(hits[0].url, "https://b.com", "标题命中 + 多源应排第一");
    }

    #[test]
    fn domain_filter_include_exclude() {
        let mut hits = vec![
            Hit::new("a".into(), "https://docs.rs/tokio".into(), "".into(), "ddg"),
            Hit::new("b".into(), "https://blog.spam.com/x".into(), "".into(), "ddg"),
            Hit::new("c".into(), "https://sub.docs.rs/y".into(), "".into(), "ddg"),
        ];
        let mut p = SearchParams::new("q", 5);
        p.include_domains = vec!["docs.rs".into()];
        apply_domain_filter(&mut hits, &p);
        assert_eq!(hits.len(), 2, "仅保留 docs.rs 及其子域");
        assert!(hits.iter().all(|h| h.url.contains("docs.rs")));
    }

    #[test]
    fn normalize_url_strips_scheme_www_trailing_and_frag() {
        assert_eq!(normalize_url("https://www.Example.com/Path/"), normalize_url("http://example.com/Path"));
        assert_eq!(normalize_url("https://x.com/a#sec"), "x.com/a");
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
