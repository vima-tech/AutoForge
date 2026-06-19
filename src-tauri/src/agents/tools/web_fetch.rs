//! 网页抓取/阅读工具——给定 URL 抓取页面并提取可读正文回灌给 Agent。
//!
//! 与 `web_search` 互补：搜索拿到「标题/链接/摘要」后，用本工具真正「打开并阅读」
//! 目标页面，而非只看搜索引擎摘要。**原生、零配置、免 API Key**。
//!
//! 设计约束（CLAUDE.md 后端独立化铁律 + 安全规则）：
//! - 纯 Rust，零 Tauri 类型；无状态、不碰 db。
//! - 结果是不可信外部输入：注入检测/截断由 [`super::ToolRegistry::invoke`] 统一施加。
//! - **SSRF 防护**：仅允许 http/https；拒绝 localhost / 内网 / 链路本地 / 元数据地址，
//!   并对重定向逐跳校验目标主机，防止被诱导抓取内部服务。
//! - 仅处理网页/纯文本/JSON/XML 内容类型，二进制（图片/PDF…）直接拒绝。

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;

/// 类浏览器 UA：部分站点（含 DuckDuckGo）对空/脚本 UA 返回空页或拦截。
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// web_fetch 工厂：免配置、任何场景可用（不依赖项目上下文）。
pub struct WebFetchFactory;

#[async_trait]
impl BuiltinTool for WebFetchFactory {
    fn info(&self) -> ToolInfo {
        ToolInfo { name: "web_fetch", label: "网页抓取阅读", needs_project: false }
    }
    async fn build(&self, _db: &Db, _ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        // 无需任何配置 / Key——原生开箱即用。
        Some(Arc::new(WebFetchTool) as Arc<dyn Tool>)
    }
}

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_fetch",
            "网页抓取阅读：给定一个 http/https 网址，打开页面并提取可读正文返回。\
             需要阅读某个链接的完整内容（文档、文章、API 文档、issue 等）时使用——\
             与 web_search 搭配：先搜索拿到链接，再用本工具读取正文。",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "要抓取的页面 URL，必须以 http:// 或 https:// 开头。"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "返回正文的最大字符数（500-16000），默认 8000。",
                        "minimum": 500,
                        "maximum": 16000
                    }
                },
                "required": ["url"]
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("缺少参数 url"))?;
        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(500, 16000))
            .unwrap_or(8000);
        let body = fetch_readable(url, max_chars).await?;
        Ok(format!("# 来源：{}\n\n{}", url.trim(), body))
    }
}

/// 抓取 URL 并返回提取后的可读正文（已截断到 `max_chars`）。
/// 供 web_fetch 工具与 web_search「搜索后自动读正文」共用。
pub async fn fetch_readable(url: &str, max_chars: usize) -> Result<String> {
    let url = url.trim();
    validate_url(url)?;
    let resp = make_client()?.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("抓取失败：HTTP {}", status.as_u16());
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_plain = ctype.contains("text/plain") || ctype.contains("json");
    let is_html = ctype.is_empty() || ctype.contains("html") || ctype.contains("xml");
    if !is_plain && !is_html {
        bail!("不支持的内容类型：{}（仅支持网页/纯文本）", ctype);
    }
    let raw = resp.text().await?;
    let text = if is_plain { collapse_blank_lines(&raw) } else { html_to_text(&raw) };
    let text = text.trim();
    if text.is_empty() {
        bail!("页面无可提取的正文");
    }
    Ok(truncate_chars(text, max_chars))
}

/// 构造带重定向逐跳校验的 HTTP 客户端：每一跳目标主机都过 SSRF 闸。
fn make_client() -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("重定向次数过多")
        } else if host_blocked(attempt.url().host_str().unwrap_or("")) {
            attempt.error("重定向目标指向内网/本地地址，已按安全策略拒绝")
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .redirect(policy)
        .build()
        .map_err(|e| anyhow!(e))
}

// ───────────────────────── SSRF 防护 ─────────────────────────

fn validate_url(url: &str) -> Result<()> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        bail!("仅支持 http/https 协议的 URL");
    }
    let host = host_of(url).ok_or_else(|| anyhow!("无法解析 URL 主机"))?;
    if host_blocked(&host) {
        bail!("出于安全策略，禁止抓取本地/内网地址（{}）", host);
    }
    Ok(())
}

/// 从 URL 中粗取主机名（不引入 url crate）：scheme://[userinfo@]host[:port]/...
fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1)?;
    let authority = after.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?; // 去 userinfo
    let host = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 字面量 [::1]:port
        let end = rest.find(']')?;
        &authority[..=end + 1]
    } else {
        authority.split(':').next()?
    };
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    (!host.is_empty()).then(|| host.to_string())
}

/// 主机是否禁止抓取（本地/内网/链路本地/特殊用途）。
fn host_blocked(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']') // IPv6 字面量去括号
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    if h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".home")
        || h.ends_with(".lan")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return ip_blocked(&ip);
    }
    false
}

fn ip_blocked(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // CGNAT 100.64.0.0/10
                || (o[0] == 100 && (o[1] & 0xc0) == 0x40)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                // ULA fc00::/7
                || (s[0] & 0xfe00) == 0xfc00
                // 链路本地 fe80::/10
                || (s[0] & 0xffc0) == 0xfe80
                // IPv4 映射地址 ::ffff:0:0/96 → 取末段按 IPv4 判定
                || v6.to_ipv4_mapped().map(|v4| ip_blocked(&IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

// ───────────────────────── HTML → 文本 ─────────────────────────

static RE_COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
/// 要整体丢弃内容的标签（regex crate 不支持反向引用，故每个标签单独编译一条）。
static RE_DROP: Lazy<Vec<Regex>> = Lazy::new(|| {
    ["script", "style", "noscript", "svg", "template", "iframe"]
        .iter()
        .map(|t| Regex::new(&format!(r"(?is)<{t}\b[^>]*>.*?</{t}\s*>")).unwrap())
        .collect()
});
static RE_BLOCK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)</?(p|div|br|li|tr|h[1-6]|section|article|header|footer|main|nav|ul|ol|table|blockquote|pre|hr)\b[^>]*>").unwrap()
});
static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static RE_INLINE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t\x0b\x0c\r]+").unwrap());
static RE_BLANK_LINES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());
static RE_NUM_ENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"&#(x?[0-9a-fA-F]+);").unwrap());

/// 把 HTML 提取为纯文本：去脚本/样式/注释 → 块级标签转行 → 去标签 → 解码实体 → 规整空白。
pub fn html_to_text(html: &str) -> String {
    let mut s = RE_COMMENT.replace_all(html, " ").into_owned();
    for re in RE_DROP.iter() {
        s = re.replace_all(&s, " ").into_owned();
    }
    let s = RE_BLOCK.replace_all(&s, "\n");
    let s = RE_TAG.replace_all(&s, " ");
    let s = decode_entities(&s);
    let s = RE_INLINE_WS.replace_all(&s, " ");
    collapse_blank_lines(&s)
}

/// 提取 HTML 片段（如搜索结果标题/摘要）为单行纯文本。
pub fn clean_inline(html: &str) -> String {
    let s = RE_TAG.replace_all(html, " ");
    let s = decode_entities(&s);
    RE_INLINE_WS.replace_all(s.trim(), " ").trim().to_string()
}

fn collapse_blank_lines(s: &str) -> String {
    let joined = s
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    RE_BLANK_LINES.replace_all(&joined, "\n\n").trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let named = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&middot;", "·")
        .replace("&hellip;", "…")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&laquo;", "«")
        .replace("&raquo;", "»");
    RE_NUM_ENT
        .replace_all(&named, |caps: &regex::Captures| {
            let raw = &caps[1];
            let code = if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                raw.parse::<u32>().ok()
            };
            code.and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
}

/// 字符安全截断（按 char 边界，避免切断多字节 UTF-8）。
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n…（内容已截断）")
}

/// 百分号解码（用于解 DuckDuckGo 重定向中的 uddg 目标 URL 等）。
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_local_and_private_hosts() {
        for h in ["localhost", "127.0.0.1", "10.1.2.3", "192.168.0.1", "169.254.169.254",
                  "172.16.5.5", "[::1]", "0.0.0.0", "foo.internal", "box.local", "100.64.1.1"] {
            assert!(host_blocked(h), "应拦截 {h}");
        }
    }

    #[test]
    fn allows_public_hosts() {
        for h in ["example.com", "8.8.8.8", "duckduckgo.com", "1.1.1.1", "203.0.114.5"] {
            assert!(!host_blocked(h), "应放行 {h}");
        }
    }

    #[test]
    fn validate_rejects_non_http_and_private() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("http://127.0.0.1:8080/admin").is_err());
        assert!(validate_url("http://localhost/x").is_err());
        assert!(validate_url("https://example.com/page").is_ok());
    }

    #[test]
    fn host_of_parses_authority() {
        assert_eq!(host_of("https://user:pw@example.com:443/p?q=1").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[::1]:9000/x").as_deref(), Some("::1"));
        assert_eq!(host_of("https://EXAMPLE.com").as_deref(), Some("EXAMPLE.com"));
    }

    #[test]
    fn html_to_text_strips_and_decodes() {
        let html = "<html><head><title>T</title><style>x{}</style></head>\
                    <body><script>alert(1)</script><h1>标题</h1><p>正文&amp;内容&#39;&#x41;</p></body></html>";
        let t = html_to_text(html);
        assert!(t.contains("标题"));
        assert!(t.contains("正文&内容'A"));
        assert!(!t.contains("alert"));
        assert!(!t.contains("x{}"));
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("https%3A%2F%2Fa.com%2Fx%20y"), "https://a.com/x y");
    }

    #[test]
    fn truncate_is_char_safe() {
        let s = "中文字符测试";
        let out = truncate_chars(s, 3);
        assert!(out.starts_with("中文字"));
        assert!(out.contains("截断"));
    }
}
