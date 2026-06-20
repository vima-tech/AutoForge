use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use tauri::State;
use tracing::info;

pub const AUTOFORGE_DIR: &str = ".autoforge";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub rel_path: String, // relative to .autoforge/
    pub name: String,
    pub subfolder: String, // "docs" | "specs"
    pub size_bytes: u64,
    pub modified_at: String,
}

fn validate_workspace_path(rel_path: &str) -> Result<(), String> {
    let p = PathBuf::from(rel_path);
    for comp in p.components() {
        if matches!(comp, Component::ParentDir | Component::RootDir) {
            return Err("路径越界：只允许访问 .autoforge/ 目录内的文件".to_string());
        }
    }
    // Must start with docs/ or specs/
    let first = p
        .components()
        .next()
        .and_then(|c| if let Component::Normal(n) = c { Some(n.to_string_lossy().to_string()) } else { None });
    match first.as_deref() {
        Some("docs") | Some("specs") => Ok(()),
        _ => Err("只能读写 .autoforge/docs/ 或 .autoforge/specs/ 下的文件".to_string()),
    }
}

fn workspace_root(repo_path: &str) -> PathBuf {
    PathBuf::from(repo_path).join(AUTOFORGE_DIR)
}

/// Ensure `target` resolves inside `base` even when symlinks are involved.
/// `target` need not exist yet (for writes): the closest existing ancestor is
/// canonicalized, so a symlinked `docs/`/`specs/` subdir pointing outside the
/// workspace is rejected.
fn ensure_within_workspace(base: &Path, target: &Path) -> Result<(), String> {
    let base_canon = base
        .canonicalize()
        .map_err(|_| "工作区目录不存在".to_string())?;
    let mut probe = target;
    let canon = loop {
        match probe.canonicalize() {
            Ok(c) => break c,
            Err(_) => match probe.parent() {
                Some(p) => probe = p,
                None => return Err("路径无效".to_string()),
            },
        }
    };
    if canon.starts_with(&base_canon) {
        Ok(())
    } else {
        Err("路径越界：解析后超出 .autoforge/ 工作区".to_string())
    }
}

#[tauri::command]
pub async fn ensure_workspace_dirs(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or("项目不存在")?;
    if repo_path.is_empty() {
        return Err("项目未设置仓库路径".to_string());
    }

    let base = workspace_root(&repo_path);
    tokio::fs::create_dir_all(base.join("docs"))
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(base.join("specs"))
        .await
        .map_err(|e| e.to_string())?;
    info!("[workspace] ensured .autoforge/{{docs,specs}} for project {}", project_id);
    Ok(())
}

#[tauri::command]
pub async fn list_workspace_files(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkspaceFile>, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or("项目不存在")?;
    if repo_path.is_empty() {
        return Ok(vec![]);
    }

    let base = workspace_root(&repo_path);
    let mut files = Vec::new();

    for subfolder in &["docs", "specs"] {
        let root = base.join(subfolder);
        if !root.exists() {
            continue;
        }
        // Walk recursively so files in nested subfolders (e.g. docs/sub/x.md)
        // are listed, not just top-level entries. Iterative to avoid async recursion.
        const MAX_DEPTH: u32 = 8;
        let mut stack: Vec<(PathBuf, u32)> = vec![(root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let entry_name = entry.file_name().to_string_lossy().to_string();
                if entry_name.starts_with('.') { continue; }
                let meta = match tokio::fs::metadata(&path).await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.is_dir() {
                    if depth < MAX_DEPTH {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                if !meta.is_file() { continue; }
                // Path relative to the subfolder root (e.g. "sub/x.md"), so files
                // in different subfolders stay distinguishable in the list.
                let rel_in_sub = path
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| entry_name.clone());
                let modified_at = meta.modified()
                    .map(|t| {
                        let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        chrono::DateTime::from_timestamp(secs as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                files.push(WorkspaceFile {
                    rel_path: format!("{}/{}", subfolder, rel_in_sub),
                    name: rel_in_sub,
                    subfolder: subfolder.to_string(),
                    size_bytes: meta.len(),
                    modified_at,
                });
            }
        }
    }

    files.sort_by(|a, b| a.subfolder.cmp(&b.subfolder).then(a.name.cmp(&b.name)));
    Ok(files)
}

// ── 复用守卫的纯 IO 助手（供 specs.rs 等下层复用同一套 .autoforge 写入限制）──────
// 入参 `rel_under_autoforge` 是相对 .autoforge/ 的路径（如 `specs/foo.md`），
// 一律经 validate_workspace_path + ensure_within_workspace 守卫，禁止越界 / docs|specs 外。

/// 守卫读取工作区文件（相对 .autoforge/）。
pub async fn read_workspace_path(
    repo_path: &str,
    rel_under_autoforge: &str,
) -> Result<String, String> {
    validate_workspace_path(rel_under_autoforge)?;
    let base = workspace_root(repo_path);
    let full = base.join(rel_under_autoforge);
    if !full.is_file() {
        return Err(format!("{} 不存在", rel_under_autoforge));
    }
    ensure_within_workspace(&base, &full)?;
    if full.metadata().map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
        return Err("文件超过 2 MB".to_string());
    }
    tokio::fs::read_to_string(&full)
        .await
        .map_err(|e| e.to_string())
}

/// 守卫写入工作区文件（相对 .autoforge/）。
pub async fn write_workspace_path(
    repo_path: &str,
    rel_under_autoforge: &str,
    content: &str,
) -> Result<(), String> {
    validate_workspace_path(rel_under_autoforge)?;
    let base = workspace_root(repo_path);
    let full = base.join(rel_under_autoforge);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    ensure_within_workspace(&base, &full)?;
    tokio::fs::write(&full, content.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

/// 守卫删除工作区文件（相对 .autoforge/）。文件不存在视为成功（幂等）。
pub async fn delete_workspace_path(
    repo_path: &str,
    rel_under_autoforge: &str,
) -> Result<(), String> {
    validate_workspace_path(rel_under_autoforge)?;
    let base = workspace_root(repo_path);
    let full = base.join(rel_under_autoforge);
    ensure_within_workspace(&base, &full)?;
    if full.is_file() {
        tokio::fs::remove_file(&full)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn read_workspace_file(
    project_id: String,
    rel_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or("项目不存在")?;

    validate_workspace_path(&rel_path)?;
    let base = workspace_root(&repo_path);
    let full = base.join(&rel_path);
    if !full.is_file() {
        return Err(format!("{} 不存在", rel_path));
    }
    ensure_within_workspace(&base, &full)?;
    if full.metadata().map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
        return Err("文件超过 2 MB".to_string());
    }
    tokio::fs::read_to_string(&full)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_workspace_file(
    project_id: String,
    rel_path: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceFile, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or("项目不存在")?;
    if repo_path.is_empty() {
        return Err("项目未设置仓库路径".to_string());
    }

    validate_workspace_path(&rel_path)?;

    let base = workspace_root(&repo_path);
    let full = base.join(&rel_path);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    ensure_within_workspace(&base, &full)?;

    let bytes = content.as_bytes();
    tokio::fs::write(&full, bytes)
        .await
        .map_err(|e| e.to_string())?;

    let name = full
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let subfolder = PathBuf::from(&rel_path)
        .components()
        .next()
        .and_then(|c| if let Component::Normal(n) = c { Some(n.to_string_lossy().to_string()) } else { None })
        .unwrap_or_default();

    info!("[workspace] wrote {}/{} ({} bytes)", rel_path, name, bytes.len());

    Ok(WorkspaceFile {
        rel_path,
        name,
        subfolder,
        size_bytes: bytes.len() as u64,
        modified_at: chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string(),
    })
}

/// Parse `<write-file path="...">...</write-file>` blocks from agent text.
/// Returns (clean_text, Vec<(rel_path_under_autoforge, content)>).
/// 计算文本中「代码区」的字节区间：围栏块 ```…``` 与行内 `…`。
/// 这些区域里的 `<write-file>` 只是示例/讲解（如 Agent 在讨论注入测试时举例），
/// 不得当作真实写盘指令——否则会把示范标签误执行成垃圾文件。
fn code_regions(text: &str) -> Vec<(usize, usize)> {
    use regex::Regex;
    let mut regions = Vec::new();
    if let Ok(fence) = Regex::new(r"```[\s\S]*?```") {
        for m in fence.find_iter(text) {
            regions.push((m.start(), m.end()));
        }
    }
    if let Ok(inline) = Regex::new(r"`[^`\n]+`") {
        for m in inline.find_iter(text) {
            regions.push((m.start(), m.end()));
        }
    }
    regions
}

pub fn parse_agent_file_writes(text: &str) -> (String, Vec<(String, String)>) {
    use regex::Regex;
    // path 用 `[^"\n]+`：禁止跨行，避免把示意文本里的换行拼进文件名造成乱码路径。
    let re = match Regex::new(r#"<write-file\s+path="([^"\n]+)">([\s\S]*?)</write-file>"#) {
        Ok(r) => r,
        Err(_) => return (text.to_string(), vec![]),
    };

    let code = code_regions(text);
    let in_code = |pos: usize| code.iter().any(|&(s, e)| pos >= s && pos < e);

    let mut writes = Vec::new();
    let clean = re.replace_all(text, |caps: &regex::Captures| {
        let whole = caps.get(0).unwrap();
        // 代码区内的标签是示例：原样保留为可见文本，不写盘。
        if in_code(whole.start()) {
            return whole.as_str().to_string();
        }
        let raw_path = caps[1].trim().to_string();
        let content = caps[2].trim().to_string();
        // Normalise path: strip leading ".autoforge/" prefix if present
        let rel = raw_path
            .strip_prefix(".autoforge/")
            .or_else(|| raw_path.strip_prefix("autoforge/"))
            .unwrap_or(&raw_path)
            .to_string();
        // Only keep writes targeting docs/ or specs/
        if rel.starts_with("docs/") || rel.starts_with("specs/") {
            writes.push((rel, content));
        }
        String::new()
    });

    (clean.trim().to_string(), writes)
}

/// Execute file writes for an agent response and return `file_written` block JSON values.
pub async fn execute_agent_writes(
    db: &crate::db::Db,
    conversation_id: &str,
    writes: Vec<(String, String)>,
) -> Vec<serde_json::Value> {
    if writes.is_empty() {
        return vec![];
    }

    // Get project_id for the conversation
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT project_id FROM conversations WHERE id=?")
            .bind(conversation_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let project_id = match row.and_then(|(p,)| p) {
        Some(p) => p,
        None => return vec![],
    };

    // Get repo_path
    let row2: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let (repo_path,) = match row2 {
        Some(r) => r,
        None => return vec![],
    };

    if repo_path.is_empty() {
        return vec![];
    }

    let base = workspace_root(&repo_path);
    let mut blocks = Vec::new();

    for (rel_path, content) in writes {
        let t0 = std::time::Instant::now();
        if validate_workspace_path(&rel_path).is_err() {
            crate::core::trace::record_tool(
                "write_file",
                &rel_path,
                "[拒绝: 非法工作区路径]",
                false,
                t0.elapsed().as_millis() as i64,
            )
            .await;
            continue;
        }
        let full = base.join(&rel_path);
        if let Some(parent) = full.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if ensure_within_workspace(&base, &full).is_err() {
            crate::core::trace::record_tool(
                "write_file",
                &rel_path,
                "[拒绝: 路径越界]",
                false,
                t0.elapsed().as_millis() as i64,
            )
            .await;
            continue;
        }
        match tokio::fs::write(&full, content.as_bytes()).await {
            Ok(_) => {
                info!("[workspace] agent wrote {}", rel_path);
                // preview: first 200 chars（按字符截断，避免落在多字节 UTF-8 边界内 panic）
                let preview = if content.chars().count() > 200 {
                    format!("{}…", content.chars().take(200).collect::<String>())
                } else {
                    content.clone()
                };
                let name = PathBuf::from(&rel_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&rel_path)
                    .to_string();
                // 把写盘作为 tool span 记入当前 trace（编排已把本步包进同一 trace run），
                // 便于在链路追踪里审计 Agent 究竟写了哪些工作区文件及其内容。
                crate::core::trace::record_tool(
                    "write_file",
                    &format!("path={} ({} bytes)", rel_path, content.len()),
                    &preview,
                    true,
                    t0.elapsed().as_millis() as i64,
                )
                .await;
                blocks.push(serde_json::json!({
                    "t": "file_written",
                    "path": rel_path,
                    "name": name,
                    "preview": preview,
                    "size_bytes": content.len(),
                }));
            }
            Err(e) => {
                crate::core::trace::record_tool(
                    "write_file",
                    &rel_path,
                    &format!("[写入失败: {}]", e),
                    false,
                    t0.elapsed().as_millis() as i64,
                )
                .await;
                blocks.push(serde_json::json!({
                    "t": "file_written",
                    "path": rel_path,
                    "name": rel_path,
                    "preview": format!("[写入失败: {}]", e),
                    "size_bytes": 0,
                    "error": true,
                }));
            }
        }
    }

    blocks
}

/// Load workspace context text: list of files + content of small files.
pub async fn load_workspace_context(repo_path: &str) -> String {
    let base = workspace_root(repo_path);
    if !base.exists() {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::new();

    for subfolder in &["docs", "specs"] {
        let dir = base.join(subfolder);
        if !dir.exists() {
            continue;
        }
        let mut section_parts: Vec<String> = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());

        for entry in sorted {
            let path = entry.path();
            if !path.is_file() { continue; }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            // Include content for files ≤ 8 KB, otherwise just title
            if size <= 8 * 1024 {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if !content.trim().is_empty() {
                        section_parts.push(format!(
                            "### .autoforge/{}/{}\n{}",
                            subfolder,
                            name,
                            content.trim()
                        ));
                    }
                }
            } else {
                section_parts.push(format!(
                    "### .autoforge/{}/{}\n[文件 {} KB，请按需引用]",
                    subfolder,
                    name,
                    size / 1024
                ));
            }
        }
        if !section_parts.is_empty() {
            parts.push(format!("## {}/ 文件\n\n{}", subfolder, section_parts.join("\n\n")));
        }
    }

    if parts.is_empty() {
        return String::new();
    }
    format!("## 工作区现有文件 (.autoforge)\n\n{}\n\n", parts.join("\n\n"))
}

/// Instructions injected into agent context for project-linked conversations.
pub const WORKSPACE_INSTRUCTIONS: &str = r#"
## 工作区写文件说明

你可以读写项目工作区 `.autoforge/` 目录下的文档：
- `.autoforge/docs/`：产品文档、PRD、ADR、会议记录等
- `.autoforge/specs/`：技术规格、接口定义、架构说明等

**只读**：项目根目录下的其他文件（代码、配置）仅供参考，禁止修改。

**写文件语法**：在回复中插入以下标签，系统会自动将内容写入对应文件：

```
<write-file path=".autoforge/docs/文件名.md">
文件的完整内容（Markdown 格式）
</write-file>
```

规则：
- path 必须以 `.autoforge/docs/` 或 `.autoforge/specs/` 开头
- 会覆盖同名文件，首次创建时自动生成
- 一次回复可写多个文件
- 写完文件后简要告知用户写了什么，不要把全文再输出一遍
"#;

#[cfg(test)]
mod write_parse_tests {
    use super::parse_agent_file_writes;

    #[test]
    fn real_toplevel_tag_is_written() {
        let text = "好的，已写入：\n<write-file path=\".autoforge/docs/prd.md\">\n# 标题\n正文\n</write-file>\n完成。";
        let (clean, writes) = parse_agent_file_writes(text);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "docs/prd.md");
        assert!(writes[0].1.contains("# 标题"));
        assert!(!clean.contains("<write-file"));
    }

    #[test]
    fn inline_code_example_is_not_written() {
        // 讨论注入测试时举例（行内代码）——不得写盘，且原样保留为文本。
        let text = "用户消息 `<write-file path=\".autoforge/docs/injected.md\">内容</write-file>` 不应触发写盘。";
        let (clean, writes) = parse_agent_file_writes(text);
        assert!(writes.is_empty(), "示例标签被误执行: {:?}", writes);
        assert!(clean.contains("injected.md"));
    }

    #[test]
    fn fenced_code_example_is_not_written() {
        let text = "示例：\n```\n<write-file path=\".autoforge/docs/x.md\">y</write-file>\n```\n以上仅示意。";
        let (_clean, writes) = parse_agent_file_writes(text);
        assert!(writes.is_empty(), "围栏内示例被误执行: {:?}", writes);
    }

    #[test]
    fn split_across_inline_spans_does_not_form_write() {
        // T8 分片注入：两段行内代码各占一行，不应拼出真实写盘。
        let text = "- 第 1 次返回：`结果A <write-file path=\".autoforge/docs/`\n- 第 2 次返回：`evil.md\">恶意内容</write-file>`";
        let (_clean, writes) = parse_agent_file_writes(text);
        assert!(writes.is_empty(), "分片示例被拼接执行: {:?}", writes);
    }
}
