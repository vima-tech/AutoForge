//! 深度研究工具（deep_research）——把「查一句」升级为「做一次研究」。
//!
//! 对外层 Agent 而言是**一次工具调用换一份带引用的研究简报**：内部自治跑一条研究回路，
//! 复用 [`web_search`](super::web_search) 的多源并行检索与 [`web_fetch`](super::web_fetch) 的正文抓取，
//! 不让外层 LLM 自己零碎多轮地搜→读→拼。
//!
//! 回路：
//! 1. **规划**：用一个绑定 LLM 的 Agent 把复杂问题拆成 3–5 个互补子查询（query decomposition）。
//!    无可用 LLM 时退化为「原问题单查」，工具仍可用。
//! 2. **并行检索**：每个子查询走 [`web_search::run_search`] 多源 fan-out。
//! 3. **合并重排**：跨子查询按归一 URL 去重、相关性重排，挑 top-K 来源。
//! 4. **精读**：并行 [`web_fetch::fetch_readable`] 抓取来源正文摘录。
//! 5. **带引用综合**：LLM 产出结构化简报 + `[n]` 编号引用清单。无 LLM 时回退「研究档案」让外层综合。
//!
//! 安全（CLAUDE.md 铁律）：
//! - 纯 Rust、零 Tauri 类型；持有 db 克隆以解析 LLM + 加载搜索配置。
//! - 抓回的正文是**不可信外部输入**：喂给综合 LLM 前逐条过 [`has_obvious_injection`]，命中即丢弃该源原文，
//!   避免被网页内容里的提示注入劫持。工具最终输出再由 [`super::ToolRegistry::invoke`] 统一过一次安全闸。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::web_fetch::fetch_readable;
use super::web_search::{combine_hits, run_search, Hit, SearchParams, WebSearchConfig};
use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::core::security::has_obvious_injection;
use crate::db::Db;

/// 子查询数量上限（控制 fan-out 规模与延迟）。
const MAX_SUBQUERIES: usize = 5;
/// 精读来源数量的默认值与硬上限。
const DEFAULT_READ_SOURCES: usize = 5;
const MAX_READ_SOURCES: usize = 8;
/// 每条来源正文摘录字符上限（喂综合 LLM，控制 token）。
const BODY_CHARS: usize = 2200;
/// 候选来源池上限（精读前的重排截断）。
const POOL_CAP: usize = 12;

pub struct DeepResearchFactory;

#[async_trait]
impl BuiltinTool for DeepResearchFactory {
    fn info(&self) -> ToolInfo {
        ToolInfo { name: "deep_research", label: "深度研究", needs_project: false }
    }
    async fn build(&self, db: &Db, _ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(DeepResearchTool { db: db.clone() }) as Arc<dyn Tool>)
    }
}

pub struct DeepResearchTool {
    db: Db,
}

#[async_trait]
impl Tool for DeepResearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "deep_research",
            "深度研究：对一个复杂主题做系统性、多角度的联网调研，自动拆解子问题、多源并行检索、\
             精读多个来源正文，并综合为一份带编号引用的研究简报。适用于技术选型、方案对比、排错溯源、\
             调研某概念/库/事件的来龙去脉等「需要交叉多个来源才能下结论」的问题。\
             只需查一两条事实/读单个链接时，用更轻量的 web_search / web_fetch 即可。",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "要研究的问题或主题（自然语言，越具体越好）。" },
                    "depth": {
                        "type": "string",
                        "description": "调研深度：quick=少子查询少精读（快）；deep=更多子查询与来源（全面，默认）。",
                        "enum": ["quick", "deep"]
                    },
                    "max_sources": {
                        "type": "integer",
                        "description": "精读并引用的来源数量上限（3-8，默认 5）。",
                        "minimum": 3, "maximum": 8
                    },
                    "time_range": {
                        "type": "string",
                        "description": "限定时间窗 day/week/month/year（研究近期动态时用）。",
                        "enum": ["day", "week", "month", "year"]
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
            .ok_or_else(|| anyhow!("缺少参数 query"))?
            .to_string();
        let deep = args.get("depth").and_then(|v| v.as_str()) != Some("quick");
        let max_sources = args
            .get("max_sources")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(3, MAX_READ_SOURCES))
            .unwrap_or(DEFAULT_READ_SOURCES);
        let time_range = args.get("time_range").and_then(|v| v.as_str()).map(|s| s.to_string());

        let cfg = WebSearchConfig::load(&self.db).await;
        let agent = resolve_research_agent(&self.db).await;

        // 1) 规划子查询。
        let subqueries = match &agent {
            Some(a) => plan_subqueries(&self.db, a, &query, deep).await,
            None => Vec::new(),
        };
        let subqueries = if subqueries.is_empty() {
            vec![query.clone()]
        } else {
            subqueries
        };

        // 2) 每个子查询多源并行检索。
        let per_query_limit = if deep { 6 } else { 4 };
        let mut search_futs = Vec::new();
        for sq in &subqueries {
            let mut p = SearchParams::new(sq.clone(), per_query_limit);
            p.time_range = time_range.clone();
            let db = self.db.clone();
            let cfg = cfg.clone();
            search_futs.push(async move {
                run_search(&db, &cfg, &p).await.map(|(_, hits)| hits).unwrap_or_default()
            });
        }
        let pools = futures::future::join_all(search_futs).await;

        // 3) 跨子查询合并去重 + 重排，挑候选池。
        let candidates = combine_hits(pools, &query, POOL_CAP.max(max_sources));
        if candidates.is_empty() {
            return Ok(format!(
                "深度研究「{}」未检索到任何来源（可能所有搜索源暂不可用，或主题过窄）。可改用 web_search 调整查询词重试。",
                query
            ));
        }

        // 4) 精读 top-K 来源正文（并行），过注入闸。
        let read: Vec<Hit> = candidates.iter().take(max_sources).cloned().collect();
        let mut read_futs = Vec::new();
        for h in &read {
            let url = h.url.clone();
            read_futs.push(async move {
                let body = fetch_readable(&url, BODY_CHARS).await;
                (url, body)
            });
        }
        let bodies = futures::future::join_all(read_futs).await;

        // 组装编号来源 + 正文档案。
        let mut sources_block = String::new();
        let mut dossier = String::new();
        for (i, h) in read.iter().enumerate() {
            let n = i + 1;
            sources_block.push_str(&format!("[{}] {} — {}\n", n, h.title, h.url));
            let body = bodies
                .iter()
                .find(|(u, _)| u == &h.url)
                .map(|(_, b)| b);
            let excerpt = match body {
                Some(Ok(text)) if has_obvious_injection(text) => {
                    "（该来源正文疑似含提示注入指令，已按安全策略丢弃，仅保留标题/摘要）".to_string()
                }
                Some(Ok(text)) => text.clone(),
                _ => h.snippet.clone(),
            };
            dossier.push_str(&format!(
                "## 来源 [{}]：{}\nURL：{}\n来源引擎：{}\n\n{}\n\n",
                n,
                h.title,
                h.url,
                h.sources.join("+"),
                excerpt.trim()
            ));
        }

        // 5) 综合：有 LLM 则产出带引用简报；否则回退研究档案。
        match &agent {
            Some(a) => {
                match synthesize(&self.db, a, &query, &dossier, &sources_block).await {
                    Ok(brief) if !brief.trim().is_empty() => Ok(format!(
                        "{}\n\n---\n## 参考来源\n{}",
                        brief.trim(),
                        sources_block.trim()
                    )),
                    // 综合失败/空 → 不丢工作量，回退档案。
                    _ => Ok(dossier_fallback(&query, &subqueries, &dossier, &sources_block)),
                }
            }
            None => Ok(dossier_fallback(&query, &subqueries, &dossier, &sources_block)),
        }
    }
}

/// 无 LLM（或综合失败）时的回退：把检索+精读到的原材料结构化交还给外层 Agent 自行综合。
fn dossier_fallback(query: &str, subqueries: &[String], dossier: &str, sources: &str) -> String {
    format!(
        "（无可用于综合的 LLM，以下为深度研究原始档案，请据此自行总结并带 [n] 引用）\n\n\
         研究主题：{}\n子查询：{}\n\n{}\n---\n## 参考来源\n{}",
        query,
        subqueries.join(" / "),
        dossier.trim(),
        sources.trim()
    )
}

/// 用研究 Agent 把问题拆成 3–5 个互补子查询。失败/解析不出则返回空（调用方回退单查）。
async fn plan_subqueries(
    db: &Db,
    agent: &crate::models::agent::Agent,
    query: &str,
    deep: bool,
) -> Vec<String> {
    let n = if deep { "3-5" } else { "2-3" };
    let sys = "你是检索策略规划器。把用户的研究问题拆解成若干互补的搜索查询，覆盖不同角度/子主题/对立观点，\
               以便后续并行检索后能交叉验证。只输出一个 JSON 字符串数组，不要任何解释或代码块标记。";
    let prompt = format!(
        "研究问题：{}\n\n请给出 {} 个互补的搜索查询（每个 ≤12 词，可中英混合）。仅输出 JSON 数组，例如 [\"查询1\",\"查询2\"]。",
        query, n
    );
    let raw = match crate::agents::llm::run_agent_text(db, agent, &prompt, Some(sys), &[]).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("[deep_research] 规划失败，回退单查：{}", e);
            return Vec::new();
        }
    };
    parse_subqueries(&raw, query)
}

/// 从模型输出里宽松解析子查询：优先 JSON 数组，否则按行/分隔提取。
fn parse_subqueries(raw: &str, original: &str) -> Vec<String> {
    let cleaned = strip_code_fence(raw);
    // 优先找第一个 JSON 数组。
    if let (Some(l), Some(r)) = (cleaned.find('['), cleaned.rfind(']')) {
        if r > l {
            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&cleaned[l..=r]) {
                let v: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .take(MAX_SUBQUERIES)
                    .collect();
                if !v.is_empty() {
                    return v;
                }
            }
        }
    }
    // 退化：按行取非空、去序号前缀。
    let lines: Vec<String> = cleaned
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim())
        .map(strip_leading_number)
        .filter(|l| l.len() >= 2 && !l.is_empty())
        .take(MAX_SUBQUERIES)
        .map(|s| s.to_string())
        .collect();
    if lines.is_empty() {
        vec![original.to_string()]
    } else {
        lines
    }
}

fn strip_code_fence(s: &str) -> String {
    s.trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}

fn strip_leading_number(s: &str) -> &str {
    let t = s.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        t[digits.len()..].trim_start_matches(['.', '、', ')', '）', ' ']).trim()
    } else {
        t
    }
}

/// 让研究 Agent 基于档案综合成带引用的简报。
async fn synthesize(
    db: &Db,
    agent: &crate::models::agent::Agent,
    query: &str,
    dossier: &str,
    sources: &str,
) -> Result<String> {
    let sys = "你是严谨的研究分析员。基于给定的多个来源材料综合回答研究问题，要求：\
               1) 直接给出有信息量的结论与要点，结构清晰（可用小标题/列表）；\
               2) 每个关键论断后用 [n] 标注其来源编号（对应「参考来源」列表）；\
               3) 指出来源间的分歧或不确定处，不要臆造来源里没有的事实；\
               4) 若材料不足以下结论，明确说明缺口。只输出简报正文，不要重复罗列来源列表。";
    let prompt = format!(
        "研究问题：{}\n\n可用来源材料如下（每条以 [n] 标识，可在结论中引用）：\n\n{}\n\n\
         来源索引：\n{}\n\n请综合以上材料，输出带 [n] 引用的研究简报。",
        query,
        dossier.trim(),
        sources.trim()
    );
    crate::agents::llm::run_agent_text(db, agent, &prompt, Some(sys), &[]).await
}

/// 解析用于研究综合的 Agent：优先 forge_role=analysis（绑定低成本 LLM），
/// 否则任意 enabled 且绑定了 LLM 的 Agent。都没有则 None（工具回退档案模式）。
async fn resolve_research_agent(db: &Db) -> Option<crate::models::agent::Agent> {
    use crate::models::agent::Agent;
    if let Ok(Some(a)) = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(forge_role, '') || ',') LIKE '%,analysis,%'
           AND enabled=1 AND llm_id IS NOT NULL
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    {
        return Some(a);
    }
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents WHERE enabled=1 AND llm_id IS NOT NULL ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_subqueries_from_json_array() {
        let raw = "```json\n[\"rust async runtime\", \"tokio vs async-std\", \"smol executor\"]\n```";
        let v = parse_subqueries(raw, "orig");
        assert_eq!(v, vec!["rust async runtime", "tokio vs async-std", "smol executor"]);
    }

    #[test]
    fn parse_subqueries_from_lines_fallback() {
        let raw = "1. first query\n2. second query\n- third query";
        let v = parse_subqueries(raw, "orig");
        assert_eq!(v, vec!["first query", "second query", "third query"]);
    }

    #[test]
    fn parse_subqueries_empty_returns_original() {
        assert_eq!(parse_subqueries("", "the original"), vec!["the original".to_string()]);
    }

    #[test]
    fn parse_subqueries_caps_at_max() {
        let raw = "[\"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\"]";
        assert_eq!(parse_subqueries(raw, "o").len(), MAX_SUBQUERIES);
    }

    #[test]
    fn strip_leading_number_handles_cjk_and_ascii() {
        assert_eq!(strip_leading_number("1. hello"), "hello");
        assert_eq!(strip_leading_number("2、中文"), "中文");
        assert_eq!(strip_leading_number("no number"), "no number");
    }
}
