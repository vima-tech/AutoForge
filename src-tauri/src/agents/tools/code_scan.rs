//! 代码扫描工具组——让群聊/直聊 Agent 按需读取项目的真实源代码，而不是只能
//! 基于注入的 CLAUDE.md / 工作区做表面分析。
//!
//! 三个**只读、无副作用**工具（符合 CLAUDE.md「MVP 只读工具」铁律）：
//! - `list_project_files`  ：列出仓库（或子目录）下的文本/代码文件清单，了解结构。
//! - `read_project_file`   ：按相对路径读取真实文件内容，支持行窗口分页。
//! - `search_project_code` ：全仓内容检索（子串或正则），返回 `相对路径:行号: 内容`。
//!
//! 纯 Rust：每个工具只持有项目根目录 `repo_root`，不碰 db、不引用任何 Tauri 类型。
//! 路径一律经 [`resolve_within`] 限定在仓库内，杜绝 `..` / 符号链接越界。
//! 返回内容是项目自有源码（半可信，与喂给编码 Agent 的源码同源），故 `reads_local_source=true`：
//! 由 [`super::ToolRegistry::invoke`] 仅做截断、**豁免注入闸**——否则审计自身安全代码会被误杀。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};

/// 代码扫描工具工厂：三个工具共用一份仓库根（来自 [`ToolContext::repo_root`]）。
/// 无项目（repo_root 为空）时 `build` 返回 None，故这些工具仅在能解析到仓库时装配。
pub enum CodeScanFactory {
    Read,
    Search,
    List,
}

#[async_trait]
impl BuiltinTool for CodeScanFactory {
    fn info(&self) -> ToolInfo {
        match self {
            CodeScanFactory::Read => ToolInfo {
                name: "read_project_file",
                label: "读项目文件",
                needs_project: true,
            },
            CodeScanFactory::Search => ToolInfo {
                name: "search_project_code",
                label: "搜项目代码",
                needs_project: true,
            },
            CodeScanFactory::List => ToolInfo {
                name: "list_project_files",
                label: "列项目文件",
                needs_project: true,
            },
        }
    }

    async fn build(&self, _db: &crate::db::Db, ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        let root = ctx.repo_root.clone()?;
        Some(match self {
            CodeScanFactory::Read => Arc::new(ReadProjectFileTool::new(root)) as Arc<dyn Tool>,
            CodeScanFactory::Search => Arc::new(SearchProjectCodeTool::new(root)) as Arc<dyn Tool>,
            CodeScanFactory::List => Arc::new(ListProjectFilesTool::new(root)) as Arc<dyn Tool>,
        })
    }
}

/// 读取/搜索时跳过的单文件上限（与 project_context 保持一致，2 MB）。
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// 目录遍历最大深度，防止超深仓库拖垮一次工具调用。
const MAX_WALK_DEPTH: u32 = 8;
/// `read_project_file` 单次默认/最大返回行数（再大也会被注册表按字符数截断）。
const READ_DEFAULT_LINES: usize = 600;
const READ_MAX_LINES: usize = 1200;
/// 一次返回的输出软上限（字符），略低于注册表硬上限以保证带得上「还有更多」提示。
const OUTPUT_SOFT_CHARS: usize = 14_000;
/// `list_project_files` / `search_project_code` 的结果条数上限。
const LIST_MAX_ENTRIES: usize = 400;
const SEARCH_MAX_RESULTS_CAP: usize = 100;

/// 遍历时忽略的目录（构建产物、依赖、VCS 元数据）。
const IGNORE_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", ".git", "__pycache__",
    ".next", ".nuxt", ".venv", "venv", ".idea", ".vscode", "coverage",
];

/// 允许读取/检索的文本扩展名（白名单，避免把二进制塞进上下文）。
fn is_text_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const EXTS: &[&str] = &[
        ".md", ".txt", ".json", ".jsonc", ".yaml", ".yml", ".toml", ".ini", ".cfg",
        ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte",
        ".rs", ".py", ".go", ".java", ".kt", ".swift", ".rb", ".php", ".cs",
        ".c", ".cpp", ".cc", ".h", ".hpp", ".m", ".mm",
        ".css", ".scss", ".less", ".html", ".xml", ".sql", ".sh", ".bash", ".zsh",
        ".gradle", ".proto", ".graphql", ".dockerfile",
    ];
    if EXTS.iter().any(|e| lower.ends_with(e)) {
        return true;
    }
    // 无扩展名的常见配置/说明文件也放行。
    matches!(
        lower.as_str(),
        "dockerfile" | "makefile" | ".gitignore" | ".env.example" | "readme"
    )
}

fn is_ignored_dir(name: &str) -> bool {
    IGNORE_DIRS.contains(&name) || (name.starts_with('.') && name != ".env.example")
}

/// 把相对路径安全解析为仓库内的绝对路径：先按组件拒绝 `..` 与绝对路径，
/// 再 canonicalize 并校验仍位于仓库根内（防符号链接逃逸）。
fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim().trim_start_matches(['/', '\\']);
    if rel.is_empty() {
        return Ok(root.to_path_buf());
    }
    let candidate = Path::new(rel);
    for comp in candidate.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(anyhow!("非法路径（不允许 .. / 绝对路径 / 盘符）: {}", rel)),
        }
    }
    let joined = root.join(candidate);
    let canon = joined
        .canonicalize()
        .map_err(|e| anyhow!("路径不存在或无法访问: {} ({})", rel, e))?;
    let root_canon = root
        .canonicalize()
        .map_err(|e| anyhow!("项目根路径无效: {}", e))?;
    if !canon.starts_with(&root_canon) {
        return Err(anyhow!("路径越界：不允许访问项目目录外的文件: {}", rel));
    }
    Ok(canon)
}

/// 递归收集 `dir` 下的文本文件相对路径（相对 `root`），按字典序、受深度与条数限制。
fn collect_files(root: &Path, dir: &Path, depth: u32, out: &mut Vec<(String, u64)>) {
    if depth > MAX_WALK_DEPTH || out.len() >= LIST_MAX_ENTRIES {
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    let mut entries: Vec<std::fs::DirEntry> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= LIST_MAX_ENTRIES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if is_ignored_dir(&name) {
                continue;
            }
            collect_files(root, &path, depth + 1, out);
        } else if path.is_file() && is_text_file(&name) {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size > MAX_FILE_BYTES {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_string_lossy().replace('\\', "/"), size));
            }
        }
    }
}

/// 把 `subdir` 解析为待扫描的文件集合：既接受目录（递归收集），也接受**单个文件**
/// （收敛到该文件本身）。模型常把文件路径误传给 `subdir`（想把范围限到一个文件），
/// 容忍它能把一次必然作废的「不是目录」报错变成有用结果，避免无谓的工具轮次浪费。
fn collect_scope(root: &Path, subdir: &str) -> Result<Vec<(String, u64)>> {
    let start = resolve_within(root, subdir)?;
    let mut files = Vec::new();
    if start.is_file() {
        // 显式点名的文件即使扩展名不在白名单也予以放行；非 UTF-8 会在读取阶段被跳过。
        let size = std::fs::metadata(&start).map(|m| m.len()).unwrap_or(0);
        if size <= MAX_FILE_BYTES {
            let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            let rel = start.strip_prefix(&root_canon).unwrap_or(start.as_path());
            files.push((rel.to_string_lossy().replace('\\', "/"), size));
        }
    } else {
        collect_files(root, &start, 0, &mut files);
    }
    Ok(files)
}

// ─────────────────────────── list_project_files ───────────────────────────

pub struct ListProjectFilesTool {
    root: PathBuf,
}

impl ListProjectFilesTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for ListProjectFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "list_project_files",
            "列出项目仓库中的源代码/文本文件清单（相对路径 + 字节大小）。用于先了解代码库结构，再决定读取或搜索哪些文件。可选 subdir 限定子目录。",
            json!({
                "type": "object",
                "properties": {
                    "subdir": {
                        "type": "string",
                        "description": "可选：限定范围（相对仓库根，如 \"src/commands\"）；传文件路径则只列该文件。留空列全仓。"
                    }
                }
            }),
        )
    }

    /// 读的是本仓库自有源码（半可信）→ 豁免注册表的注入闸，仅截断不丢弃。
    fn reads_local_source(&self) -> bool {
        true
    }

    async fn call(&self, args: Value) -> Result<String> {
        let subdir = args.get("subdir").and_then(|v| v.as_str()).unwrap_or("");
        let files = collect_scope(&self.root, subdir)?;
        if files.is_empty() {
            return Ok(format!(
                "{} 下未找到可读的文本/代码文件。",
                if subdir.is_empty() { "项目根" } else { subdir }
            ));
        }
        let truncated = files.len() >= LIST_MAX_ENTRIES;
        let mut out = format!("项目文件清单（共 {} 个{}）：\n", files.len(), if truncated { "，已达上限" } else { "" });
        for (rel, size) in &files {
            out.push_str(&format!("{}\t{} B\n", rel, size));
        }
        if truncated {
            out.push_str("\n（结果已截断，请用 subdir 缩小范围查看更多。）");
        }
        Ok(out)
    }
}

// ──────────────────────────── read_project_file ───────────────────────────

pub struct ReadProjectFileTool {
    root: PathBuf,
}

impl ReadProjectFileTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for ReadProjectFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "read_project_file",
            "读取项目仓库中某个文件的真实内容（带行号），用于获取准确实现细节而非泛泛而谈。文件较大时用 start_line/max_lines 分页，结果会提示总行数与如何续读。",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件相对仓库根的路径，如 \"src-tauri/src/agents/llm.rs\"。"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "起始行号（1 起，含）。默认 1。",
                        "minimum": 1
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "最多返回行数，默认 600，上限 1200。",
                        "minimum": 1
                    }
                },
                "required": ["path"]
            }),
        )
    }

    /// 读的是本仓库自有源码（半可信）→ 豁免注册表的注入闸，仅截断不丢弃。
    fn reads_local_source(&self) -> bool {
        true
    }

    async fn call(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("缺少参数 path"))?;
        let full = resolve_within(&self.root, path)?;
        if !full.is_file() {
            return Err(anyhow!("不是文件: {}", path));
        }
        let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_BYTES {
            return Err(anyhow!("文件过大（{} B，上限 {} B），请改用 search_project_code 定位片段。", size, MAX_FILE_BYTES));
        }
        let content = std::fs::read_to_string(&full)
            .map_err(|e| anyhow!("读取失败（可能是二进制文件）: {}", e))?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let start = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(1);
        if start > total && total > 0 {
            return Err(anyhow!("start_line={} 超过文件总行数 {}。", start, total));
        }
        let max_lines = args
            .get("max_lines")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, READ_MAX_LINES))
            .unwrap_or(READ_DEFAULT_LINES);

        let start_idx = start.saturating_sub(1);
        let mut out = String::new();
        let mut last_shown = start_idx;
        for (i, line) in lines.iter().enumerate().skip(start_idx) {
            if i - start_idx >= max_lines || out.len() >= OUTPUT_SOFT_CHARS {
                break;
            }
            out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
            last_shown = i + 1;
        }
        let header = format!("文件: {}（共 {} 行）\n", path, total);
        if last_shown < total {
            Ok(format!(
                "{}{}\n…[文件还有更多内容] 已显示第 {}–{} 行；如需后续内容，用 start_line={} 再次调用。",
                header, out.trim_end(), start, last_shown, last_shown + 1
            ))
        } else {
            Ok(format!("{}{}", header, out.trim_end()))
        }
    }
}

// ─────────────────────────── search_project_code ──────────────────────────

pub struct SearchProjectCodeTool {
    root: PathBuf,
}

impl SearchProjectCodeTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for SearchProjectCodeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "search_project_code",
            "在项目仓库中全文检索（默认大小写不敏感子串，可选正则），返回每个命中的 相对路径:行号: 内容。用于定位某符号/字符串在真实代码中的所有出现位置。",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "要搜索的字符串；regex=true 时按正则解析。"
                    },
                    "subdir": {
                        "type": "string",
                        "description": "可选：限定范围（相对仓库根）；传文件路径则只在该文件内检索。"
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "是否按正则匹配，默认 false（子串匹配）。"
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "是否区分大小写，默认 false。"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "最多返回命中条数，默认 40，上限 100。",
                        "minimum": 1
                    }
                },
                "required": ["query"]
            }),
        )
    }

    /// 读的是本仓库自有源码（半可信）→ 豁免注册表的注入闸，仅截断不丢弃。
    fn reads_local_source(&self) -> bool {
        true
    }

    async fn call(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("缺少参数 query"))?;
        let subdir = args.get("subdir").and_then(|v| v.as_str()).unwrap_or("");
        let is_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, SEARCH_MAX_RESULTS_CAP))
            .unwrap_or(40);

        let files = collect_scope(&self.root, subdir)?;

        // 构造匹配器：正则用 regex 引擎，子串用（按需小写化的）contains。
        let matcher = if is_regex {
            let re = regex::RegexBuilder::new(query)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|e| anyhow!("正则非法: {}", e))?;
            Matcher::Regex(re)
        } else if case_sensitive {
            Matcher::Substr(query.to_string())
        } else {
            Matcher::SubstrCi(query.to_ascii_lowercase())
        };

        let mut hits = Vec::new();
        let mut scanned_files = 0usize;
        for (rel, _size) in &files {
            if hits.len() >= max_results {
                break;
            }
            let full = self.root.join(rel);
            let content = match std::fs::read_to_string(&full) {
                Ok(c) => c,
                Err(_) => continue, // 二进制 / 非 UTF-8，跳过
            };
            scanned_files += 1;
            for (lineno, line) in content.lines().enumerate() {
                if hits.len() >= max_results {
                    break;
                }
                if matcher.is_match(line) {
                    let trimmed = line.trim();
                    let shown: String = trimmed.chars().take(240).collect();
                    hits.push(format!("{}:{}: {}", rel, lineno + 1, shown));
                }
            }
        }

        if hits.is_empty() {
            return Ok(format!(
                "在 {} 个文件中未找到「{}」。",
                scanned_files, query
            ));
        }
        let capped = hits.len() >= max_results;
        let mut out = format!("「{}」命中 {} 处{}：\n", query, hits.len(), if capped { "（已达上限，可能还有更多）" } else { "" });
        out.push_str(&hits.join("\n"));
        Ok(out)
    }
}

enum Matcher {
    Regex(regex::Regex),
    Substr(String),
    SubstrCi(String),
}

impl Matcher {
    fn is_match(&self, line: &str) -> bool {
        match self {
            Matcher::Regex(re) => re.is_match(line),
            Matcher::Substr(q) => line.contains(q.as_str()),
            Matcher::SubstrCi(q) => line.to_ascii_lowercase().contains(q.as_str()),
        }
    }
}
