//! 配置驱动的代码情报预查（push 式）。
//!
//! 执行代码实现前，AutoForge 把分析阶段定位到的符号喂给一个**可配置的 MCP 代码情报
//! 提供者**（`mcp_servers` 表里 `role='code_intel'` 的那条），取回定义位置/调用者，归一化
//! 后注入实现 prompt。三家 code agent（claude/codex/opencode）只是收到更丰富的 prompt，
//! 行为零差异——这是「统一 MCP 接入」而非给每个 CLI 各配 MCP。
//!
//! 为什么 push（AutoForge 查并注入）而非 pull（agent 自查）：查询由 AutoForge 在**主仓**跑
//! （索引在主仓 `.codegraph/`），worktree 里没有；且免去各 CLI 的 MCP 配置/`--print` 权限差异。
//!
//! 为什么不再硬编码 codegraph：提供者 = 一条 MCP 配置 + 能力映射（capability_map_json）。
//! 换工具只改配置，零 Rust 改动。codegraph 退化为一条默认种子配置（迁移 0060）。
//!
//! 支持的能力槽位（由 push 流程消费，缺则跳过该项）：
//!   - `locate_symbol`（必需）：定位符号 → file:line + 签名
//!   - `find_callers`（可选）：直接调用者
//!   - `impact_analysis`（可选）：改动波及面（遍历依赖，供编码 Agent 改前评估破坏性）
//! 能力映射形如：
//! ```json
//! { "locate_symbol":   {"tool":"codegraph_search","args":{"query":"$SYMBOL","projectPath":"$REPO","limit":1}},
//!   "find_callers":    {"tool":"codegraph_callers","args":{"symbol":"$SYMBOL","projectPath":"$REPO","limit":5}},
//!   "impact_analysis": {"tool":"codegraph_impact","args":{"symbol":"$SYMBOL","projectPath":"$REPO","depth":2}} }
//! ```
//! args 内 `$SYMBOL` / `$REPO` 为占位符，调用前替换为真实符号名 / 仓库路径。
//!
//! 安全：MCP 工具结果是不可信外部输入。push 路径绕过了 `ToolRegistry::invoke` 的统一消毒，
//! 故本模块在注入前自行过 `has_obvious_injection` 并截断（符合 CLAUDE.md 安全铁律）。
//! 提供者不可用 / 未配置 / 无符号可查 → 返回空串，绝不阻断流水线。

use crate::agents::analysis::IssueAnalysisSpec;
use crate::agents::tools::mcp::McpConnection;
use crate::models::mcp_server::McpServer;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::time::Duration;

/// 单次 MCP 工具调用的超时（best-effort，到点放弃该项）。
const CALL_TIMEOUT: Duration = Duration::from_secs(8);
/// 最多预查多少个符号（控制调用数与 prompt 体量）。
const MAX_SYMBOLS: usize = 6;
/// 为前若干个符号附带调用者（更贵，限量）。
const MAX_CALLERS_LOOKUP: usize = 3;
/// 为前若干个符号附带影响面分析（最贵：遍历依赖，限量给最可疑的几个符号）。
const MAX_IMPACT_LOOKUP: usize = 3;
/// 单条情报文本归一化后的字符上限，避免撑爆 prompt。
const MAX_SNIPPET_CHARS: usize = 600;

/// 一个能力（locate_symbol / find_callers …）的绑定：调用哪个工具、带什么参数模板。
struct Capability {
    tool: String,
    args_template: Value,
}

/// 从 `capability_map_json` 解析出能力映射。形状不符的项被跳过（容错）。
fn parse_capabilities(json: &str) -> HashMap<String, Capability> {
    let mut out = HashMap::new();
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else {
        return out;
    };
    for (cap, spec) in map {
        let Some(tool) = spec.get("tool").and_then(|v| v.as_str()) else {
            continue;
        };
        if tool.trim().is_empty() {
            continue;
        }
        let args_template = spec.get("args").cloned().unwrap_or(Value::Object(Default::default()));
        out.insert(
            cap,
            Capability {
                tool: tool.to_string(),
                args_template,
            },
        );
    }
    out
}

/// 约定发现：能力映射留空时，从 server 自报的工具表按命名+参数 schema 推断出
/// locate_symbol / find_callers 的工具与参数。这样 codegraph 这类常见代码情报工具
/// **零配置即可用**，只有非常规工具才需在「高级」里手填映射。
fn discover_capabilities(tools: &[rmcp::model::Tool]) -> HashMap<String, Capability> {
    let mut out = HashMap::new();

    // locate：名字含 search/query/lookup/find（但排除 caller/callee）的工具，取第一个。
    if let Some(t) = tools.iter().find(|t| {
        let n = t.name.to_lowercase();
        (n.contains("search") || n.contains("query") || n.contains("lookup") || n.contains("find"))
            && !n.contains("caller")
            && !n.contains("callee")
    }) {
        if let Some(cap) = build_discovered(t, &["query", "symbol", "name", "q", "term", "search"]) {
            out.insert("locate_symbol".to_string(), cap);
        }
    }

    // callers：名字含 caller 的工具。
    if let Some(t) = tools.iter().find(|t| t.name.to_lowercase().contains("caller")) {
        if let Some(cap) = build_discovered(t, &["symbol", "name", "function", "query"]) {
            out.insert("find_callers".to_string(), cap);
        }
    }

    // impact：名字含 impact 的工具（改动波及面，供编码 Agent 避免破坏性改动）。
    if let Some(t) = tools.iter().find(|t| t.name.to_lowercase().contains("impact")) {
        if let Some(cap) = build_discovered(t, &["symbol", "name", "function", "query"]) {
            out.insert("impact_analysis".to_string(), cap);
        }
    }

    out
}

/// 给发现到的工具构造参数模板：从其 input schema 里挑「符号参数」(填 $SYMBOL) 与可选的
/// 「项目路径参数」(填 $REPO)。挑不到符号参数则放弃该能力（无从调用）。
fn build_discovered(t: &rmcp::model::Tool, symbol_candidates: &[&str]) -> Option<Capability> {
    let schema: &Map<String, Value> = &t.input_schema;
    let sym_arg = pick_arg(schema, symbol_candidates)
        .or_else(|| first_string_prop(schema))?;
    let mut args = Map::new();
    args.insert(sym_arg, Value::String("$SYMBOL".to_string()));
    // 项目路径参数可选：有则注入 $REPO（让查询指向主仓索引），无则不传。
    if let Some(repo_arg) = pick_arg(
        schema,
        &["projectPath", "project_path", "path", "cwd", "root", "dir", "repo"],
    ) {
        args.insert(repo_arg, Value::String("$REPO".to_string()));
    }
    Some(Capability {
        tool: t.name.to_string(),
        args_template: Value::Object(args),
    })
}

/// 按候选名（不区分大小写）在 schema.properties 里挑第一个匹配的属性名。
fn pick_arg(schema: &Map<String, Value>, candidates: &[&str]) -> Option<String> {
    let props = schema.get("properties").and_then(|v| v.as_object())?;
    for cand in candidates {
        for key in props.keys() {
            if key.eq_ignore_ascii_case(cand) {
                return Some(key.clone());
            }
        }
    }
    None
}

/// 兜底：取 schema 里第一个 string 类型属性名（候选全不匹配时用）。
fn first_string_prop(schema: &Map<String, Value>) -> Option<String> {
    let props = schema.get("properties").and_then(|v| v.as_object())?;
    props
        .iter()
        .find(|(_, v)| v.get("type").and_then(|t| t.as_str()) == Some("string"))
        .map(|(k, _)| k.clone())
}

/// 把能力映射序列化回 `capability_map_json` 的规范形状 `{cap:{tool,args}}`。
fn capabilities_to_json(caps: &HashMap<String, Capability>) -> String {
    let obj: Map<String, Value> = caps
        .iter()
        .map(|(k, c)| {
            (
                k.clone(),
                serde_json::json!({ "tool": c.tool, "args": c.args_template }),
            )
        })
        .collect();
    serde_json::to_string_pretty(&Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

/// 连接提供者、按约定发现能力，并序列化成 `capability_map_json` 文本（供 UI 回填）。
/// 连接失败 / 未发现 → 返回 "{}"。供「测试连接 / 保存」时把约定结果显式落到配置框。
pub async fn discover_capability_map(server: &McpServer) -> String {
    let conn = match McpConnection::connect(server).await {
        Ok(c) => c,
        Err(_) => return "{}".to_string(),
    };
    let Ok(tools) = conn.list_tools().await else {
        return "{}".to_string();
    };
    let caps = discover_capabilities(&tools);
    if caps.is_empty() {
        return "{}".to_string();
    }
    capabilities_to_json(&caps)
}

/// 递归把模板里的 `$SYMBOL` / `$REPO` 字符串占位符替换为真实值（精确等值替换，
/// 避免误伤包含 `$` 的内容）。非字符串叶子原样保留。
fn fill_template(t: &Value, symbol: &str, repo: &str) -> Value {
    match t {
        Value::String(s) => {
            let replaced = match s.as_str() {
                "$SYMBOL" => symbol.to_string(),
                "$REPO" => repo.to_string(),
                other => other.to_string(),
            };
            Value::String(replaced)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fill_template(x, symbol, repo)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), fill_template(v, symbol, repo)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 轻量归一化：尽量从 MCP 返回里抽出 file:line（+签名）；抽不到就原样裁剪。
/// 兼容两类返回：① JSON 数组 `[{node:{filePath,startLine,signature,...}}]`（codegraph -j 风格）；
/// ② 任意文本（多数 MCP 工具返回格式化文本）——trim + 截断后直接用。
fn normalize(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 尝试结构化抽取（容错：失败就走文本兜底）。
    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(trimmed) {
        let mut lines = Vec::new();
        for item in arr.iter().take(5) {
            let node = item.get("node").unwrap_or(item);
            let Some(file) = node.get("filePath").and_then(|v| v.as_str()) else {
                continue;
            };
            let line = node.get("startLine").and_then(|v| v.as_i64()).unwrap_or(0);
            let kind = node.get("kind").and_then(|v| v.as_str()).unwrap_or("symbol");
            let sig = node
                .get("signature")
                .and_then(|v| v.as_str())
                .and_then(|s| s.lines().next())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            match sig {
                Some(s) => lines.push(format!("`{}:{}` — {} `{}`", file, line, kind, s)),
                None => lines.push(format!("`{}:{}` — {}", file, line, kind)),
            }
        }
        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
    }
    // 文本兜底：截断到上限。
    Some(cap_chars(trimmed, MAX_SNIPPET_CHARS))
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{}…", head)
    }
}

/// 从分析 spec 收集可查询的符号名（去重、限量、保序）。
fn collect_symbols(spec: &IssueAnalysisSpec) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    if let Some(rc) = spec.root_cause.as_ref() {
        for loc in &rc.suspected_locations {
            if let Some(sym) = loc.symbol.as_deref() {
                let sym = sym.trim();
                if !sym.is_empty() && seen.insert(sym.to_string()) {
                    out.push(sym.to_string());
                    if out.len() >= MAX_SYMBOLS {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// 取启用的、适用于编码 Agent 的代码情报提供者（按 created_at 取第一条）。无则 None。
async fn load_provider(db: &crate::db::Db) -> Option<McpServer> {
    sqlx::query_as::<_, McpServer>(
        "SELECT * FROM mcp_servers WHERE for_code_agent=1 AND enabled=1 ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// 调用一个能力，返回归一化后的安全文本（None = 无结果 / 超时 / 命中注入特征而丢弃）。
async fn invoke_capability(
    conn: &McpConnection,
    cap: &Capability,
    symbol: &str,
    repo: &str,
) -> Option<String> {
    let args = fill_template(&cap.args_template, symbol, repo);
    let raw = match tokio::time::timeout(CALL_TIMEOUT, conn.call_tool(&cap.tool, args)).await {
        Ok(Ok(text)) => text,
        _ => return None,
    };
    // MCP 结果不可信：命中明显注入特征则整条丢弃，不回灌。
    if crate::core::security::has_obvious_injection(&raw) {
        return None;
    }
    normalize(&raw)
}

/// 主入口：返回注入 prompt 的「代码定位」段。无提供者 / 无符号 / 全部失败 → 空串。
pub async fn locate_context(db: &crate::db::Db, repo_path: &str, spec: &IssueAnalysisSpec) -> String {
    let symbols = collect_symbols(spec);
    if symbols.is_empty() {
        return String::new();
    }
    let Some(server) = load_provider(db).await else {
        return String::new();
    };

    // 连接一次，复用给所有符号查询。连接失败（提供者未装等）→ 优雅退化为空。
    let conn = match McpConnection::connect(&server).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    // 能力映射：显式配置优先；留空则按工具命名 + 参数 schema 约定自动发现。
    let mut caps = parse_capabilities(&server.capability_map_json);
    if caps.is_empty() {
        if let Ok(tools) = conn.list_tools().await {
            caps = discover_capabilities(&tools);
        }
    }
    let Some(locate) = caps.get("locate_symbol") else {
        return String::new(); // 配置/发现均无定位能力 → 无从预查。
    };
    let find_callers = caps.get("find_callers");
    let impact = caps.get("impact_analysis");

    let mut blocks: Vec<String> = Vec::new();
    for (i, sym) in symbols.iter().enumerate() {
        let Some(def) = invoke_capability(&conn, locate, sym, repo_path).await else {
            continue;
        };
        let mut block = format!("### {}\n- 定位：\n{}", sym, indent(&def));
        if i < MAX_CALLERS_LOOKUP {
            if let Some(fc) = find_callers {
                if let Some(callers) = invoke_capability(&conn, fc, sym, repo_path).await {
                    block.push_str(&format!("\n- 调用者：\n{}", indent(&callers)));
                }
            }
        }
        if i < MAX_IMPACT_LOOKUP {
            if let Some(im) = impact {
                if let Some(aff) = invoke_capability(&conn, im, sym, repo_path).await {
                    block.push_str(&format!("\n- 影响面（改动波及，改前评估）：\n{}", indent(&aff)));
                }
            }
        }
        blocks.push(block);
    }

    if blocks.is_empty() {
        return String::new();
    }

    format!(
        "\n## 代码定位（{} 预查，已精确到文件:行）\n\
         > 下列位置由代码索引（MCP code-intel）预先定位。请优先直接打开这些文件/行处理，避免全仓 grep 重新摸索。\n\n{}\n",
        server.name,
        blocks.join("\n\n")
    )
}

/// 给多行文本加两空格缩进，落在 markdown 列表项下保持层级。
fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {}", l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_capabilities_skips_malformed() {
        let json = r#"{"locate_symbol":{"tool":"s","args":{"query":"$SYMBOL"}},
                       "bad":{"args":{}},"empty_tool":{"tool":""}}"#;
        let caps = parse_capabilities(json);
        assert!(caps.contains_key("locate_symbol"));
        assert!(!caps.contains_key("bad")); // 缺 tool
        assert!(!caps.contains_key("empty_tool")); // tool 空
    }

    #[test]
    fn pick_arg_matches_candidates_case_insensitively() {
        let schema: Map<String, Value> = serde_json::from_str::<Value>(
            r#"{"properties":{"Query":{"type":"string"},"projectPath":{"type":"string"},"limit":{"type":"number"}}}"#,
        )
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(pick_arg(&schema, &["query"]).as_deref(), Some("Query"));
        assert_eq!(pick_arg(&schema, &["projectPath", "path"]).as_deref(), Some("projectPath"));
        assert_eq!(pick_arg(&schema, &["symbol"]), None);
        // 候选不中时兜底到第一个 string 属性。
        assert_eq!(first_string_prop(&schema).as_deref(), Some("Query"));
    }

    #[test]
    fn parse_capabilities_supports_impact_slot() {
        // 三槽位（含新增 impact_analysis）都应被识别，工具名/参数正确解析。
        let json = r#"{
            "locate_symbol":{"tool":"codegraph_search","args":{"query":"$SYMBOL"}},
            "find_callers":{"tool":"codegraph_callers","args":{"symbol":"$SYMBOL"}},
            "impact_analysis":{"tool":"codegraph_impact","args":{"symbol":"$SYMBOL","depth":2}}
        }"#;
        let caps = parse_capabilities(json);
        assert_eq!(caps.get("impact_analysis").map(|c| c.tool.as_str()), Some("codegraph_impact"));
        assert_eq!(caps["impact_analysis"].args_template["depth"], 2);
        assert_eq!(caps.len(), 3);
    }

    #[test]
    fn capabilities_to_json_roundtrips_through_parse() {
        // 发现侧序列化出的 JSON，必须能被读取侧 parse_capabilities 解析回来（格式自洽）。
        let mut caps = HashMap::new();
        caps.insert(
            "locate_symbol".to_string(),
            Capability {
                tool: "codegraph_search".to_string(),
                args_template: serde_json::json!({"query": "$SYMBOL", "projectPath": "$REPO"}),
            },
        );
        let json = capabilities_to_json(&caps);
        let parsed = parse_capabilities(&json);
        let c = parsed.get("locate_symbol").expect("locate_symbol 应存在");
        assert_eq!(c.tool, "codegraph_search");
        assert_eq!(c.args_template["query"], "$SYMBOL");
        assert_eq!(c.args_template["projectPath"], "$REPO");
    }

    #[test]
    fn fill_template_substitutes_placeholders() {
        let t: Value = serde_json::from_str(
            r#"{"query":"$SYMBOL","projectPath":"$REPO","limit":1,"nested":["$SYMBOL","x"]}"#,
        )
        .unwrap();
        let filled = fill_template(&t, "build_prompt", "/repo");
        assert_eq!(filled["query"], "build_prompt");
        assert_eq!(filled["projectPath"], "/repo");
        assert_eq!(filled["limit"], 1);
        assert_eq!(filled["nested"][0], "build_prompt");
        assert_eq!(filled["nested"][1], "x");
    }

    #[test]
    fn normalize_extracts_json_nodes() {
        let raw = r#"[{"node":{"kind":"function","filePath":"a.rs","startLine":85,"signature":"(x:i32)->String\nmore"}}]"#;
        let out = normalize(raw).unwrap();
        assert!(out.contains("`a.rs:85`"));
        assert!(out.contains("function"));
        assert!(out.contains("(x:i32)->String"));
        assert!(!out.contains("more")); // 签名只取首行
    }

    #[test]
    fn normalize_falls_back_to_text_and_caps() {
        assert!(normalize("   ").is_none());
        let out = normalize("plain text result").unwrap();
        assert_eq!(out, "plain text result");
        let long = "x".repeat(MAX_SNIPPET_CHARS + 50);
        let capped = normalize(&long).unwrap();
        assert!(capped.ends_with('…'));
        assert!(capped.chars().count() <= MAX_SNIPPET_CHARS + 1);
    }
}
