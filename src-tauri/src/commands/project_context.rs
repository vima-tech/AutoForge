use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Component, PathBuf};
use tauri::State;
use tracing::warn;

const MAX_PROJECT_FILE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB per file

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContextFile {
    pub rel_path: String,
    pub name: String,
    pub size_bytes: u64,
    pub is_priority: bool, // claude.md / agents.md
    pub pinned: bool,      // user has pinned this file for this conversation
}

fn is_priority(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "claude.md" | "agents.md" | "claude.txt" | "agents.txt")
}

fn is_allowed_ext(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".md", ".txt", ".json", ".yaml", ".yml", ".toml", ".ts", ".tsx",
        ".js", ".jsx", ".rs", ".py", ".go", ".java", ".kt", ".swift",
        ".c", ".cpp", ".h", ".hpp", ".css", ".html", ".sql", ".sh",
        ".env.example", ".gitignore",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

fn safe_join(base: &std::path::Path, rel: &str) -> Result<PathBuf, String> {
    let joined = base.join(rel);
    let canon = joined
        .canonicalize()
        .map_err(|e| format!("路径不存在: {}", e))?;
    let base_canon = base
        .canonicalize()
        .map_err(|e| format!("项目路径无效: {}", e))?;
    if !canon.starts_with(&base_canon) {
        return Err("路径越界：不允许访问项目目录外的文件".to_string());
    }
    Ok(canon)
}

/// Walk project directory up to depth 3, collecting text files.
/// Priority files (claude.md, agents.md) appear first.
fn walk_project_files(repo_path: &std::path::Path, depth: u32) -> Vec<(String, u64)> {
    if depth > 3 {
        return vec![];
    }
    let mut results = Vec::new();
    let dir = match std::fs::read_dir(repo_path) {
        Ok(d) => d,
        Err(_) => return results,
    };
    let mut entries: Vec<std::fs::DirEntry> = dir.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common noise
        if name.starts_with('.') && name != ".env.example" {
            continue;
        }
        if matches!(
            name.as_str(),
            "node_modules" | "target" | "dist" | "build" | ".git" | "__pycache__"
        ) {
            continue;
        }

        if path.is_dir() && depth < 3 {
            let prefix = if depth == 0 {
                name.clone()
            } else {
                format!(
                    "{}/{}",
                    repo_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(""),
                    name
                )
            };
            // Recurse with relative paths built correctly elsewhere
            let sub = walk_project_files_with_prefix(&path, &prefix, depth + 1);
            results.extend(sub);
        } else if path.is_file() && is_allowed_ext(&name) {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size <= MAX_PROJECT_FILE_BYTES {
                results.push((name, size));
            }
        }
    }
    results
}

fn walk_project_files_with_prefix(
    dir: &std::path::Path,
    prefix: &str,
    depth: u32,
) -> Vec<(String, u64)> {
    if depth > 3 {
        return vec![];
    }
    let mut results = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(d) => d,
        Err(_) => return results,
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') && name != ".env.example" {
            continue;
        }
        if matches!(
            name.as_str(),
            "node_modules" | "target" | "dist" | "build" | ".git" | "__pycache__"
        ) {
            continue;
        }

        let rel = format!("{}/{}", prefix, name);
        if path.is_dir() && depth < 3 {
            let sub = walk_project_files_with_prefix(&path, &rel, depth + 1);
            results.extend(sub);
        } else if path.is_file() && is_allowed_ext(&name) {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size <= MAX_PROJECT_FILE_BYTES {
                results.push((rel, size));
            }
        }
    }
    results
}

#[tauri::command]
pub async fn list_project_files(
    project_id: String,
    conversation_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectContextFile>, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or_else(|| format!("project {} not found", project_id))?;
    if repo_path.is_empty() {
        return Ok(vec![]);
    }
    let base = PathBuf::from(&repo_path);
    if !base.exists() {
        return Ok(vec![]);
    }

    // Load pinned paths for this conversation
    let pinned_paths: Vec<String> = if let Some(ref cid) = conversation_id {
        sqlx::query_as::<_, (String,)>(
            "SELECT rel_path FROM conversation_project_context WHERE conversation_id=?",
        )
        .bind(cid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(p,)| p)
        .collect()
    } else {
        vec![]
    };

    let raw = walk_project_files(&base, 0);
    let mut files: Vec<ProjectContextFile> = raw
        .into_iter()
        .map(|(rel_path, size_bytes)| {
            let name = PathBuf::from(&rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&rel_path)
                .to_string();
            let priority = is_priority(&name);
            let pinned = priority || pinned_paths.contains(&rel_path);
            ProjectContextFile {
                rel_path,
                name,
                size_bytes,
                is_priority: priority,
                pinned,
            }
        })
        .collect();

    // Sort: priority first, then pinned, then alphabetical
    files.sort_by(|a, b| {
        b.is_priority
            .cmp(&a.is_priority)
            .then(b.pinned.cmp(&a.pinned))
            .then(a.rel_path.cmp(&b.rel_path))
    });

    Ok(files)
}

#[tauri::command]
pub async fn read_project_file(
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
    let (repo_path,) = row.ok_or_else(|| format!("project {} not found", project_id))?;
    if repo_path.is_empty() {
        return Err("项目未设置仓库路径".to_string());
    }
    read_repo_file(&repo_path, &rel_path)
}

/// 守卫读取仓库内一个文件（防越界 + 白名单后缀 + 2MB 上限）。供蓝图/孵化台引用项目文件复用。
pub fn read_repo_file(repo_path: &str, rel_path: &str) -> Result<String, String> {
    for comp in PathBuf::from(rel_path).components() {
        if matches!(comp, Component::ParentDir | Component::RootDir) {
            return Err("路径越界：不允许访问项目目录外的文件".to_string());
        }
    }
    let base = PathBuf::from(repo_path);
    let full_path = safe_join(&base, rel_path)?;
    if !is_allowed_ext(rel_path) {
        return Err("该文件类型不允许读取".to_string());
    }
    let meta = std::fs::metadata(&full_path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_PROJECT_FILE_BYTES {
        return Err(format!("文件超过 2 MB 上限（{} KB）", meta.len() / 1024));
    }
    std::fs::read_to_string(&full_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_conversation_project_context(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let paths: Vec<(String,)> = sqlx::query_as(
        "SELECT rel_path FROM conversation_project_context WHERE conversation_id=? ORDER BY added_at",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(paths.into_iter().map(|(p,)| p).collect())
}

#[tauri::command]
pub async fn add_conversation_project_context(
    conversation_id: String,
    rel_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Validate conversation exists and is a group with a project
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT type, project_id FROM conversations WHERE id=?")
            .bind(&conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (conv_type, project_id) = row.ok_or("对话不存在")?;
    if conv_type != "group" {
        return Err("只有群聊才能绑定项目上下文文件".to_string());
    }
    let project_id = project_id.ok_or("该群聊未绑定项目")?;

    // Validate file exists in project
    let row2: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (repo_path,) = row2.ok_or("项目不存在")?;

    for comp in PathBuf::from(&rel_path).components() {
        if matches!(comp, Component::ParentDir | Component::RootDir) {
            return Err("路径越界".to_string());
        }
    }
    let base = PathBuf::from(&repo_path);
    let full = safe_join(&base, &rel_path)?;
    if !full.is_file() {
        return Err("文件不存在".to_string());
    }

    sqlx::query(
        "INSERT OR IGNORE INTO conversation_project_context (conversation_id, rel_path) VALUES (?, ?)",
    )
    .bind(&conversation_id)
    .bind(&rel_path)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn remove_conversation_project_context(
    conversation_id: String,
    rel_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM conversation_project_context WHERE conversation_id=? AND rel_path=?",
    )
    .bind(&conversation_id)
    .bind(&rel_path)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Load project context text to prepend to the conversation snapshot.
/// Returns formatted string with auto-loaded priority files + pinned files.
pub async fn load_project_context_for_conversation(
    db: &crate::db::Db,
    conversation_id: &str,
) -> String {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT project_id FROM conversations WHERE id=?")
            .bind(conversation_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();

    let project_id = match row.and_then(|(pid,)| pid) {
        Some(pid) => pid,
        None => return String::new(),
    };

    let row2: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();

    let (repo_path,) = match row2 {
        Some(r) => r,
        None => return String::new(),
    };

    if repo_path.is_empty() {
        return String::new();
    }
    let base = PathBuf::from(&repo_path);

    let mut parts: Vec<String> = Vec::new();

    // 1. Auto-load priority files
    for candidate in &[
        "claude.md",
        "CLAUDE.md",
        "agents.md",
        "AGENTS.md",
        "claude.txt",
        "agents.txt",
    ] {
        let full = base.join(candidate);
        if full.is_file() {
            match std::fs::read_to_string(&full) {
                Ok(content) if !content.trim().is_empty() => {
                    parts.push(format!(
                        "=== 项目配置文件: {} ===\n{}",
                        candidate,
                        content.trim()
                    ));
                }
                Err(e) => warn!("[project_context] failed to read {}: {}", candidate, e),
                _ => {}
            }
        }
    }

    // 2. Load pinned files
    let pinned: Vec<(String,)> = sqlx::query_as(
        "SELECT rel_path FROM conversation_project_context WHERE conversation_id=? ORDER BY added_at",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    for (rel_path,) in pinned {
        // Skip if already loaded as priority
        let name = PathBuf::from(&rel_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if is_priority(&name) {
            continue;
        }

        // Resolve safely: reject traversal / symlink escape (the previous
        // component loop `continue`d the inner loop and was a no-op).
        let full = match safe_join(&base, &rel_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !full.is_file() {
            continue;
        }
        if std::fs::metadata(&full).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_PROJECT_FILE_BYTES {
            continue;
        }
        match std::fs::read_to_string(&full) {
            Ok(content) if !content.trim().is_empty() => {
                parts.push(format!(
                    "=== 项目文件: {} ===\n{}",
                    rel_path,
                    content.trim()
                ));
            }
            Err(e) => warn!("[project_context] failed to read {}: {}", rel_path, e),
            _ => {}
        }
    }

    // 3. Existing workspace files. The write-rule instructions are kept separate
    //    so they always survive the context budget enforced below.
    let workspace_ctx = crate::commands::workspace::load_workspace_context(&repo_path).await;
    if !workspace_ctx.is_empty() {
        parts.push(workspace_ctx);
    }
    let instructions = crate::commands::workspace::WORKSPACE_INSTRUCTIONS;

    // 4. 规格注入（统一来源：project_specs，含 db 内联与 file 指针）。
    //    - always 的 file 源规格 → 全文随上下文（db 的 always 已由 CLAUDE.md @import 覆盖，不重复）。
    //    - on_demand 规格（任意来源）→ 仅紧凑清单，Agent 用 read_spec 工具按需拉全文。
    let (always_specs_text, on_demand_catalog) =
        build_spec_injection(db, &project_id).await;
    if !always_specs_text.is_empty() {
        parts.push(always_specs_text);
    }

    if parts.is_empty() && on_demand_catalog.is_empty() {
        return format!("## 项目上下文\n\n{}\n\n---\n", instructions);
    }

    // Cap the assembled file content to a total byte budget so a large CLAUDE.md
    // or several pinned files can't bloat every agent prompt (this whole block is
    // re-sent to every agent on every step/round). Priority files (claude.md /
    // agents.md) are first, so truncation drops the least-critical tail first.
    const MAX_PROJECT_CTX_BYTES: usize = 16 * 1024;
    let body = truncate_to_bytes(&parts.join("\n\n"), MAX_PROJECT_CTX_BYTES);

    // on_demand 清单接在预算外（与 instructions 同级，体积小、必须保留），
    // 确保 Agent 始终知道有哪些规格可用 read_spec 拉取。
    let catalog_block = if on_demand_catalog.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", on_demand_catalog)
    };

    format!(
        "## 项目上下文\n\n{}{}\n\n{}\n\n---\n",
        body, catalog_block, instructions
    )
}

/// 构造规格注入两部分：(always 的 file 源规格全文, on_demand 规格紧凑清单)。
/// db 源的 always 规格不在此返回——它们已经由 CLAUDE.md `@import` 聚合文件注入，避免重复。
async fn build_spec_injection(db: &crate::db::Db, project_id: &str) -> (String, String) {
    let specs = sqlx::query_as::<_, crate::models::spec::ProjectSpec>(
        "SELECT * FROM project_specs
         WHERE project_id = ? AND injection IN ('always','on_demand')
         ORDER BY category, sort_order, created_at",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    if specs.is_empty() {
        return (String::new(), String::new());
    }

    // 取项目仓库根，用于读 file 源 always 规格全文。
    let repo_path: String = sqlx::query_as::<_, (String,)>(
        "SELECT repo_path FROM projects WHERE id = ?",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(p,)| p)
    .unwrap_or_default();

    let mut always_parts: Vec<String> = Vec::new();
    let mut catalog_lines: Vec<String> = Vec::new();

    for s in &specs {
        if s.injection == "always" && s.source == "file" {
            let rel = s
                .rel_path
                .trim()
                .trim_start_matches("./")
                .strip_prefix(".autoforge/")
                .unwrap_or(s.rel_path.trim())
                .to_string();
            if let Ok(content) =
                crate::commands::workspace::read_workspace_path(&repo_path, &rel).await
            {
                if !content.trim().is_empty() {
                    always_parts.push(format!("=== 项目规格: {} ===\n{}", s.title, content.trim()));
                }
            }
        } else if s.injection == "on_demand" {
            let desc = if s.description.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", s.description.trim())
            };
            catalog_lines.push(format!("- [{}] {}{} (id={})", s.category, s.title, desc, s.id));
        }
    }

    let always_text = always_parts.join("\n\n");
    let catalog = if catalog_lines.is_empty() {
        String::new()
    } else {
        format!(
            "## 可用规格（按需用 read_spec 工具读取全文，勿凭空臆测）\n{}",
            catalog_lines.join("\n")
        )
    };
    (always_text, catalog)
}

/// Truncate a string to at most `max_bytes`, on a char boundary, appending a
/// notice when content was dropped.
fn truncate_to_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n…[项目上下文过长，已截断]", &s[..end])
}
