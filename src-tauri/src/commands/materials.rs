use crate::models::material::{MaterialBackupConfig, MaterialFile, MaterialFolder};
use crate::state::{self, AppState};
use base64::{engine::general_purpose, Engine as _};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::State;
use tracing::warn;
use uuid::Uuid;

const MAX_MATERIAL_BYTES: usize = 100 * 1024 * 1024; // 100 MB
const REPO_MATERIAL_PREFIX: &str = "repo://";
const MATERIAL_AI_SYSTEM_KIND: &str = "material_ai";
const OTHER_MATERIAL_FOLDER: &str = "其他";
const FIXED_MATERIAL_FOLDERS: &[(&str, &str)] = &[
    ("产品需求", "PRD、需求说明、用户故事、功能清单"),
    ("用户研究", "访谈记录、调研资料、竞品分析、用户反馈"),
    ("设计文档", "交互稿、视觉稿、原型、设计规范"),
    ("技术规格", "架构说明、接口文档、数据模型、技术约束"),
    ("测试验收", "测试用例、验收标准、缺陷记录、质量报告"),
    ("品牌素材", "Logo、品牌图片、图标、宣传素材"),
    ("项目管理", "会议记录、排期计划、里程碑、归档资料"),
    (OTHER_MATERIAL_FOLDER, "无法明确归入以上分类的文件"),
];

// ── helper structs for AI organize ────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct OrgAssignment {
    file_index: usize,
    folder_name: String,
}

#[derive(serde::Deserialize)]
struct OrgPlan {
    #[serde(default)]
    assignments: Vec<OrgAssignment>,
    summary: String,
}

#[derive(Serialize)]
pub struct MaterialSearchResult {
    file: MaterialFile,
    score: i64,
    match_reason: String,
    content_preview: Option<String>,
    folder_path: String,
}

#[derive(serde::Deserialize)]
struct AiSearchItem {
    file_id: String,
    reason: String,
}

#[derive(serde::Deserialize)]
struct AiSearchPlan {
    results: Vec<AiSearchItem>,
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn safe_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn fixed_material_folder_list() -> String {
    FIXED_MATERIAL_FOLDERS
        .iter()
        .map(|(name, desc)| format!("- {}：{}", name, desc))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_material_folder_name(
    name: &str,
    existing_folder_labels: &std::collections::HashSet<String>,
) -> String {
    let trimmed = name.trim();
    if existing_folder_labels.contains(trimmed) {
        return trimmed.to_string();
    }

    for (fixed, _) in FIXED_MATERIAL_FOLDERS {
        if trimmed == *fixed {
            return fixed.to_string();
        }
    }

    match trimmed {
        "需求文档" | "产品文档" | "功能需求" => "产品需求".to_string(),
        "调研资料" | "竞品分析" | "用户反馈" => "用户研究".to_string(),
        "设计稿" | "设计素材" | "原型设计" | "视觉设计" => "设计文档".to_string(),
        "架构文档" | "接口文档" | "开发文档" => "技术规格".to_string(),
        "测试用例" | "测试报告" | "验收文档" => "测试验收".to_string(),
        "品牌资产" | "品牌资源" | "图片素材" | "图标素材" => "品牌素材".to_string(),
        "会议记录" | "项目计划" | "排期计划" | "归档" => "项目管理".to_string(),
        _ => OTHER_MATERIAL_FOLDER.to_string(),
    }
}

fn materials_dir(project_id: &str) -> PathBuf {
    PathBuf::from(state::materials_base()).join(project_id)
}

fn material_disk_path(file: &MaterialFile) -> PathBuf {
    if let Some(path) = file.rel_path.strip_prefix(REPO_MATERIAL_PREFIX) {
        PathBuf::from(path)
    } else {
        PathBuf::from(state::materials_base()).join(&file.rel_path)
    }
}

fn is_repo_material(file: &MaterialFile) -> bool {
    file.rel_path.starts_with(REPO_MATERIAL_PREFIX)
}

fn normalize_search_text(value: &str) -> String {
    value.to_lowercase()
}

fn read_text_preview(file: &MaterialFile, max_bytes: usize) -> Option<String> {
    if !is_text_mime(&file.mime) {
        return None;
    }

    let path = material_disk_path(file);
    let mut opened = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max_bytes];
    let read = opened.read(&mut buf).ok()?;
    buf.truncate(read);
    String::from_utf8(buf).ok()
}

fn folder_path_label(
    folder_id: Option<&str>,
    folder_map: &std::collections::HashMap<String, MaterialFolder>,
) -> String {
    let Some(folder_id) = folder_id else {
        return "根目录".to_string();
    };

    let mut names: Vec<String> = Vec::new();
    let mut cursor = Some(folder_id.to_string());
    while let Some(id) = cursor {
        let Some(folder) = folder_map.get(&id) else {
            break;
        };
        names.push(folder.name.clone());
        cursor = folder.parent_id.clone();
    }
    names.reverse();

    if names.is_empty() {
        "根目录".to_string()
    } else {
        names.join(" / ")
    }
}

fn guess_mime(path: &Path) -> String {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn unique_child_path(dir: &Path, file_name: &str) -> PathBuf {
    let safe_name = safe_filename(file_name);
    let candidate = dir.join(&safe_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(&safe_name);
    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("file");
    let ext = path.extension().and_then(|v| v.to_str());

    for i in 1..10_000 {
        let name = match ext {
            Some(ext) if !ext.is_empty() => format!("{}-{}.{}", stem, i, ext),
            _ => format!("{}-{}", stem, i),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(format!("{}_{}", Uuid::new_v4(), safe_name))
}

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// 将文件/目录移入系统回收站（而非永久删除），便于误删恢复。
/// 失败时返回错误而不静默回退到永久删除——宁可保留文件，也不绕过回收站。
fn move_path_to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("移入回收站失败：{}", e))
}

/// Ensure `.autoforge/docs/` exists on disk and sync its contents into the DB.
/// Files directly in `.autoforge/docs/` get `folder_id = NULL`.
/// Sub-directories become top-level `material_folders` (`parent_id = NULL`).
async fn ensure_and_sync_docs(project_id: &str, state: &AppState) -> Result<(), String> {
    let Some(docs_dir) = project_docs_dir(project_id, state).await? else {
        return Ok(());
    };
    sync_repo_docs(project_id, &docs_dir, state).await
}

async fn get_or_create_material_folder(
    project_id: &str,
    parent_id: Option<&str>,
    name: &str,
    state: &AppState,
) -> Result<MaterialFolder, String> {
    let existing = match parent_id {
        Some(parent_id) => {
            sqlx::query_as::<_, MaterialFolder>(
                "SELECT * FROM material_folders
                 WHERE project_id = ? AND parent_id = ? AND lower(name) = lower(?)
                 ORDER BY sort_order, name, created_at, id
                 LIMIT 1",
            )
            .bind(project_id)
            .bind(parent_id)
            .bind(name)
            .fetch_optional(&state.db)
            .await
        }
        None => {
            sqlx::query_as::<_, MaterialFolder>(
                "SELECT * FROM material_folders
                 WHERE project_id = ? AND parent_id IS NULL AND lower(name) = lower(?)
                 ORDER BY sort_order, name, created_at, id
                 LIMIT 1",
            )
            .bind(project_id)
            .bind(name)
            .fetch_optional(&state.db)
            .await
        }
    }
    .map_err(|e| e.to_string())?;

    if let Some(folder) = existing {
        return Ok(folder);
    }

    let id = Uuid::new_v4().to_string();
    let now = now_str();
    sqlx::query(
        "INSERT INTO material_folders (id, project_id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&parent_id)
    .bind(name)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialFolder>("SELECT * FROM material_folders WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Walk `.autoforge/docs/` and upsert its contents into the DB.
/// Items directly in docs_dir → `folder_id = NULL`.
/// Sub-directories → top-level `material_folders` (`parent_id = NULL`).
async fn sync_repo_docs(project_id: &str, docs_dir: &Path, state: &AppState) -> Result<(), String> {
    // (dir_on_disk, db_folder_id)  None means the docs root itself
    let mut pending: Vec<(PathBuf, Option<String>)> = vec![(docs_dir.to_path_buf(), None)];

    while let Some((dir, folder_id)) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    "[materials] failed to read docs dir {}: {}",
                    dir.display(),
                    e
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.is_empty() || name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                let parent_id = folder_id.as_deref();
                let folder =
                    get_or_create_material_folder(project_id, parent_id, &name, state).await?;
                pending.push((path, Some(folder.id)));
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let metadata = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("[materials] failed to stat {}: {}", path.display(), e);
                    continue;
                }
            };
            if metadata.len() as usize > MAX_MATERIAL_BYTES {
                warn!(
                    "[materials] skip large file {} ({} bytes)",
                    path.display(),
                    metadata.len()
                );
                continue;
            }

            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    warn!("[materials] failed to read {}: {}", path.display(), e);
                    continue;
                }
            };
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let sha256 = hex::encode(hasher.finalize());
            let disk_path = std::fs::canonicalize(&path).unwrap_or(path.clone());
            let rel_path = format!("{}{}", REPO_MATERIAL_PREFIX, disk_path.to_string_lossy());
            let mime = guess_mime(&path);
            let now = now_str();

            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM material_files WHERE project_id = ? AND rel_path = ?",
            )
            .bind(project_id)
            .bind(&rel_path)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;

            if let Some((id,)) = existing {
                sqlx::query(
                    "UPDATE material_files
                     SET folder_id = ?, original_name = ?, stored_name = ?, mime = ?,
                         size_bytes = ?, sha256 = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(&folder_id)
                .bind(&name)
                .bind(&name)
                .bind(&mime)
                .bind(metadata.len() as i64)
                .bind(&sha256)
                .bind(&now)
                .bind(&id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
            } else {
                let id = Uuid::new_v4().to_string();
                sqlx::query(
                    "INSERT INTO material_files
                     (id, project_id, folder_id, original_name, stored_name, rel_path,
                      mime, size_bytes, sha256, tags, backup_status, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', 'none', ?, ?)",
                )
                .bind(&id)
                .bind(project_id)
                .bind(&folder_id)
                .bind(&name)
                .bind(&name)
                .bind(&rel_path)
                .bind(&mime)
                .bind(metadata.len() as i64)
                .bind(&sha256)
                .bind(&now)
                .bind(&now)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

async fn project_docs_dir(project_id: &str, state: &AppState) -> Result<Option<PathBuf>, String> {
    let Some((repo_path,)) =
        sqlx::query_as::<_, (String,)>("SELECT repo_path FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };

    let repo_path = repo_path.trim();
    if repo_path.is_empty() {
        return Ok(None);
    }

    let repo_dir = PathBuf::from(repo_path);
    if !repo_dir.is_dir() {
        return Ok(None);
    }

    let docs_dir = repo_dir.join(".autoforge").join("docs");
    std::fs::create_dir_all(&docs_dir).map_err(|e| e.to_string())?;
    Ok(Some(docs_dir))
}

/// Returns the disk path for the given folder_id under `.autoforge/docs/`.
/// `folder_id = None` → `.autoforge/docs/` itself.
async fn folder_path_from_docs(
    project_id: &str,
    folder_id: Option<&str>,
    state: &AppState,
) -> Result<Option<PathBuf>, String> {
    let Some(mut path) = project_docs_dir(project_id, state).await? else {
        return Ok(None);
    };
    let Some(folder_id) = folder_id else {
        return Ok(Some(path));
    };

    let mut chain: Vec<MaterialFolder> = Vec::new();
    let mut cursor = Some(folder_id.to_string());
    while let Some(id) = cursor {
        let Some(folder) = sqlx::query_as::<_, MaterialFolder>(
            "SELECT * FROM material_folders WHERE id = ? AND project_id = ?",
        )
        .bind(&id)
        .bind(project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        cursor = folder.parent_id.clone();
        chain.push(folder);
    }

    for folder in chain.iter().rev() {
        let segment = safe_filename(&folder.name);
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    Ok(Some(path))
}

fn build_folder_disk_path_in_memory(
    folder_id: &str,
    folder_map: &std::collections::HashMap<String, MaterialFolder>,
    docs_dir: &Path,
) -> Option<PathBuf> {
    let mut chain: Vec<&MaterialFolder> = Vec::new();
    let mut cursor = Some(folder_id.to_string());
    while let Some(id) = cursor {
        let folder = folder_map.get(&id)?;
        cursor = folder.parent_id.clone();
        chain.push(folder);
    }
    let mut path = docs_dir.to_path_buf();
    for folder in chain.iter().rev() {
        let segment = safe_filename(&folder.name);
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    Some(path)
}

async fn sync_deleted_folders(project_id: &str, state: &AppState) -> Result<(), String> {
    let Some(docs_dir) = project_docs_dir(project_id, state).await? else {
        return Ok(());
    };

    let all_folders: Vec<MaterialFolder> = sqlx::query_as::<_, MaterialFolder>(
        "SELECT * FROM material_folders WHERE project_id = ? ORDER BY sort_order, name",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let folder_map: std::collections::HashMap<String, MaterialFolder> = all_folders
        .iter()
        .map(|f| (f.id.clone(), f.clone()))
        .collect();

    for folder in &all_folders {
        // May have been removed in a previous iteration (ancestor deletion cascades)
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM material_folders WHERE id = ?")
            .bind(&folder.id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        if count == 0 {
            continue;
        }

        if let Some(disk_path) =
            build_folder_disk_path_in_memory(&folder.id, &folder_map, &docs_dir)
        {
            if !disk_path.exists() {
                sqlx::query(
                    "WITH RECURSIVE tree(id) AS (
                       SELECT id FROM material_folders WHERE id = ?
                       UNION ALL
                       SELECT mf.id FROM material_folders mf JOIN tree ON mf.parent_id = tree.id
                     )
                     DELETE FROM material_files WHERE folder_id IN (SELECT id FROM tree)",
                )
                .bind(&folder.id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;

                sqlx::query(
                    "WITH RECURSIVE tree(id) AS (
                       SELECT id FROM material_folders WHERE id = ?
                       UNION ALL
                       SELECT mf.id FROM material_folders mf JOIN tree ON mf.parent_id = tree.id
                     )
                     DELETE FROM material_folders WHERE id IN (SELECT id FROM tree)",
                )
                .bind(&folder.id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

async fn cleanup_material_file_records(project_id: &str, state: &AppState) -> Result<(), String> {
    let files: Vec<MaterialFile> = sqlx::query_as::<_, MaterialFile>(
        "SELECT * FROM material_files
         WHERE project_id = ?
         ORDER BY updated_at DESC, created_at DESC, id DESC",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut seen_rel_paths = std::collections::HashSet::new();
    for file in files {
        let disk_path = material_disk_path(&file);
        if !disk_path.exists() {
            sqlx::query("DELETE FROM material_files WHERE id = ?")
                .bind(&file.id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
            continue;
        }

        if !seen_rel_paths.insert(file.rel_path.clone()) {
            sqlx::query("DELETE FROM material_files WHERE id = ?")
                .bind(&file.id)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

// ── Folder CRUD ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_material_folders(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<MaterialFolder>, String> {
    ensure_and_sync_docs(&project_id, &state).await?;
    sync_deleted_folders(&project_id, &state).await?;
    cleanup_material_file_records(&project_id, &state).await?;

    sqlx::query_as::<_, MaterialFolder>(
        "SELECT * FROM material_folders WHERE project_id = ? ORDER BY sort_order, name",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_material_folder(
    project_id: String,
    parent_id: Option<String>,
    name: String,
    state: State<'_, AppState>,
) -> Result<MaterialFolder, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("文件夹名不能为空".into());
    }

    let parent_id = parent_id.filter(|s| !s.is_empty());

    let folder =
        get_or_create_material_folder(&project_id, parent_id.as_deref(), &name, &state).await?;

    if let Some(path) = folder_path_from_docs(&project_id, Some(&folder.id), &state).await? {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }

    Ok(folder)
}

#[tauri::command]
pub async fn rename_material_folder(
    id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<MaterialFolder, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("文件夹名不能为空".into());
    }
    sqlx::query("UPDATE material_folders SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(now_str())
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialFolder>("SELECT * FROM material_folders WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_material_folder(
    id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let Some(folder) =
        sqlx::query_as::<_, MaterialFolder>("SELECT * FROM material_folders WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };

    let folder_path = folder_path_from_docs(&folder.project_id, Some(&id), &state).await?;

    // 子树内的所有文件记录：用于回收非 repo（app 数据目录）物料 + 级联清理 DB。
    let files: Vec<MaterialFile> = sqlx::query_as::<_, MaterialFile>(
        "WITH RECURSIVE folder_tree(id) AS (
           SELECT id FROM material_folders WHERE id = ?
           UNION ALL
           SELECT mf.id FROM material_folders mf
           JOIN folder_tree ft ON mf.parent_id = ft.id
         )
         SELECT * FROM material_files WHERE folder_id IN (SELECT id FROM folder_tree)",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // 1) 整个文件夹目录连同内容一并移入系统回收站（覆盖目录内所有 repo 物料）。
    if let Some(path) = folder_path.as_ref() {
        if path.exists() {
            move_path_to_trash(path)?;
        }
    }

    // 2) 目录未覆盖到的文件（非 repo / app 数据目录物料）逐个移入回收站。
    //    repo 物料已随上面的目录被回收，此处 disk 不再存在会自动跳过。
    for file in &files {
        let disk = material_disk_path(file);
        if disk.exists() {
            move_path_to_trash(&disk)?;
        }
    }

    // 3) 级联删除 DB 记录：先删子树内文件，再删子树内文件夹（含所有子文件夹）。
    sqlx::query(
        "WITH RECURSIVE folder_tree(id) AS (
           SELECT id FROM material_folders WHERE id = ?
           UNION ALL
           SELECT mf.id FROM material_folders mf
           JOIN folder_tree ft ON mf.parent_id = ft.id
         )
         DELETE FROM material_files WHERE folder_id IN (SELECT id FROM folder_tree)",
    )
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "WITH RECURSIVE folder_tree(id) AS (
           SELECT id FROM material_folders WHERE id = ?
           UNION ALL
           SELECT mf.id FROM material_folders mf
           JOIN folder_tree ft ON mf.parent_id = ft.id
         )
         DELETE FROM material_folders WHERE id IN (SELECT id FROM folder_tree)",
    )
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(true)
}

// ── File CRUD ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_material_files(
    project_id: String,
    folder_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<MaterialFile>, String> {
    ensure_and_sync_docs(&project_id, &state).await?;
    sync_deleted_folders(&project_id, &state).await?;
    cleanup_material_file_records(&project_id, &state).await?;

    let rows = match folder_id.as_deref() {
        Some(fid) if !fid.is_empty() => {
            sqlx::query_as::<_, MaterialFile>(
                "SELECT * FROM material_files WHERE project_id = ? AND folder_id = ? ORDER BY created_at DESC",
            )
            .bind(&project_id)
            .bind(fid)
            .fetch_all(&state.db)
            .await
        }
        _ => {
            sqlx::query_as::<_, MaterialFile>(
                "SELECT * FROM material_files WHERE project_id = ? ORDER BY created_at DESC",
            )
            .bind(&project_id)
            .fetch_all(&state.db)
            .await
        }
    };
    rows.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_material_files(
    project_id: String,
    query: String,
    ai_mode: bool,
    state: State<'_, AppState>,
) -> Result<Vec<MaterialSearchResult>, String> {
    ensure_and_sync_docs(&project_id, &state).await?;
    sync_deleted_folders(&project_id, &state).await?;
    cleanup_material_file_records(&project_id, &state).await?;

    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let files = sqlx::query_as::<_, MaterialFile>(
        "SELECT * FROM material_files WHERE project_id = ? ORDER BY updated_at DESC",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let folders = sqlx::query_as::<_, MaterialFolder>(
        "SELECT * FROM material_folders WHERE project_id = ? ORDER BY sort_order, name",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let folder_map: std::collections::HashMap<String, MaterialFolder> =
        folders.into_iter().map(|f| (f.id.clone(), f)).collect();

    let q = normalize_search_text(&query);
    let terms: Vec<String> = q
        .split_whitespace()
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect();
    let terms = if terms.is_empty() {
        vec![q.clone()]
    } else {
        terms
    };

    let mut base_results: Vec<MaterialSearchResult> = Vec::new();
    for file in &files {
        let name = normalize_search_text(&file.original_name);
        let desc = normalize_search_text(file.description.as_deref().unwrap_or(""));
        let tags = normalize_search_text(&file.tags);
        let mime = normalize_search_text(&file.mime);
        let folder_path = folder_path_label(file.folder_id.as_deref(), &folder_map);
        let folder_text = normalize_search_text(&folder_path);
        let preview = read_text_preview(file, 24 * 1024);
        let preview_text = preview
            .as_deref()
            .map(normalize_search_text)
            .unwrap_or_default();

        let mut score = 0i64;
        let mut reasons: Vec<&str> = Vec::new();
        for term in &terms {
            if name.contains(term) {
                score += 45;
                reasons.push("文件名匹配");
            }
            if desc.contains(term) {
                score += 38;
                reasons.push("用途说明匹配");
            }
            if tags.contains(term) {
                score += 32;
                reasons.push("标签匹配");
            }
            if folder_text.contains(term) {
                score += 20;
                reasons.push("文件夹匹配");
            }
            if preview_text.contains(term) {
                score += 24;
                reasons.push("内容匹配");
            }
            if mime.contains(term) {
                score += 8;
                reasons.push("类型匹配");
            }
        }

        if score > 0 {
            reasons.sort_unstable();
            reasons.dedup();
            base_results.push(MaterialSearchResult {
                file: file.clone(),
                score,
                match_reason: reasons.join("、"),
                content_preview: preview.map(|text| text.chars().take(420).collect()),
                folder_path,
            });
        }
    }

    base_results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.file.updated_at.cmp(&a.file.updated_at))
    });

    if !ai_mode {
        base_results.truncate(50);
        return Ok(base_results);
    }

    const AI_SNIPPET_CHARS: usize = 1200;
    let mut file_entries = Vec::with_capacity(files.len());
    for file in &files {
        let snippet = read_text_preview(file, 16 * 1024)
            .map(|text| text.chars().take(AI_SNIPPET_CHARS).collect::<String>())
            .unwrap_or_else(|| "[非文本文件或内容不可读]".to_string());
        let entry = format!(
            "file_id: {}\n文件名: {}\n类型: {}\n位置: {}\n用途说明: {}\n标签: {}\n内容片段:\n{}",
            file.id,
            file.original_name,
            file.mime,
            folder_path_label(file.folder_id.as_deref(), &folder_map),
            file.description.as_deref().unwrap_or(""),
            file.tags,
            snippet
        );
        file_entries.push(entry);
    }

    let prompt = format!(
        r#"你是物料库文件检索助手。根据用户问题，从文件名、用途说明、标签、文件位置和内容片段判断哪些文件最相关。

用户要找：{}

文件候选：
{}

严格只输出 JSON，不要输出 Markdown：
{{
  "results": [
    {{"file_id": "文件 id", "reason": "为什么这个文件匹配，20字以内"}}
  ]
}}

要求：
- 最多返回 12 个结果，按相关性从高到低排序
- 可根据文件实际作用、内容语义、近义表达判断，不要求关键词完全一致
- 只返回上方候选中存在的 file_id
- 没有相关文件时返回空 results"#,
        query,
        file_entries.join("\n\n---\n\n")
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        MATERIAL_AI_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的物料库 AI 助手，负责基于文件元数据和内容片段完成语义检索与整理。只输出调用方要求的格式。"),
        Some(&project_id),
        Some(&query), // 召回键直接用用户搜索词，命中本项目相关物料经验
    )
        .await
        .map_err(|e| format!("AI 查找失败: {}", e))?;
    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回格式错误，JSON 不完整")?;
    let plan: AiSearchPlan = serde_json::from_str(&raw[start..=end])
        .map_err(|e| format!("解析 AI 查找结果失败: {}", e))?;

    let file_map: std::collections::HashMap<String, MaterialFile> =
        files.into_iter().map(|f| (f.id.clone(), f)).collect();
    let mut results = Vec::new();
    for (idx, item) in plan.results.into_iter().take(12).enumerate() {
        let Some(file) = file_map.get(&item.file_id) else {
            continue;
        };
        results.push(MaterialSearchResult {
            file: file.clone(),
            score: 10_000 - idx as i64,
            match_reason: item.reason,
            content_preview: read_text_preview(file, 16 * 1024)
                .map(|text| text.chars().take(420).collect()),
            folder_path: folder_path_label(file.folder_id.as_deref(), &folder_map),
        });
    }

    Ok(results)
}

#[tauri::command]
pub async fn import_material_file(
    project_id: String,
    folder_id: Option<String>,
    file_name: String,
    mime_hint: String,
    data_base64: String,
    state: State<'_, AppState>,
) -> Result<MaterialFile, String> {
    let raw = general_purpose::STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("base64 解码错误: {}", e))?;

    if raw.len() > MAX_MATERIAL_BYTES {
        return Err(format!(
            "文件过大，最大支持 {} MB",
            MAX_MATERIAL_BYTES / 1024 / 1024
        ));
    }

    let mut hasher = Sha256::new();
    hasher.update(&raw);
    let sha256 = hex::encode(hasher.finalize());

    let id = Uuid::new_v4().to_string();
    let safe_name = safe_filename(&file_name);
    let mime = if mime_hint.is_empty() {
        guess_mime(Path::new(&safe_name))
    } else {
        mime_hint
    };
    let now = now_str();
    // folder_id=None → docs root; folder_id=Some(id) → subdirectory
    let folder_id = folder_id.filter(|s| !s.is_empty());

    let (stored_name, rel_path) = if let Some(dir) =
        folder_path_from_docs(&project_id, folder_id.as_deref(), &state).await?
    {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = unique_child_path(&dir, &safe_name);
        std::fs::write(&dest, &raw).map_err(|e| e.to_string())?;
        let disk_path = std::fs::canonicalize(&dest).unwrap_or(dest.clone());
        let sname = dest
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(&safe_name)
            .to_string();
        (
            sname,
            format!("{}{}", REPO_MATERIAL_PREFIX, disk_path.to_string_lossy()),
        )
    } else {
        // No repo path configured — fall back to app data dir
        let stored_name = format!("{}_{}", id, safe_name);
        let dir = materials_dir(&project_id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(&stored_name), &raw).map_err(|e| e.to_string())?;
        (
            stored_name.clone(),
            format!("{}/{}", project_id, stored_name),
        )
    };

    if let Some((existing_id,)) = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM material_files WHERE project_id = ? AND rel_path = ?",
    )
    .bind(&project_id)
    .bind(&rel_path)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    {
        sqlx::query(
            "UPDATE material_files
             SET folder_id = ?, original_name = ?, stored_name = ?, mime = ?,
                 size_bytes = ?, sha256 = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&folder_id)
        .bind(&file_name)
        .bind(&stored_name)
        .bind(&mime)
        .bind(raw.len() as i64)
        .bind(&sha256)
        .bind(&now)
        .bind(&existing_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        return sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
            .bind(&existing_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }

    sqlx::query(
        "INSERT INTO material_files
         (id, project_id, folder_id, original_name, stored_name, rel_path,
          mime, size_bytes, sha256, tags, backup_status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', 'none', ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&folder_id)
    .bind(&file_name)
    .bind(&stored_name)
    .bind(&rel_path)
    .bind(&mime)
    .bind(raw.len() as i64)
    .bind(&sha256)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_material_file(
    id: String,
    folder_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<MaterialFile, String> {
    let file = sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let folder_id = folder_id.filter(|s| !s.is_empty());
    let mut rel_path = file.rel_path.clone();
    let mut stored_name = file.stored_name.clone();

    if let Some(dir) = folder_path_from_docs(&file.project_id, folder_id.as_deref(), &state).await?
    {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let safe_name = safe_filename(&file.original_name);
        let dest = dir.join(&safe_name);
        let src = material_disk_path(&file);
        if src.exists() {
            if dest.exists() && dest != src {
                std::fs::remove_file(&dest).map_err(|e| e.to_string())?;
            }
            if src != dest {
                match std::fs::rename(&src, &dest) {
                    Ok(_) => {}
                    Err(_) => {
                        std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
                        if !is_repo_material(&file) {
                            let _ = std::fs::remove_file(&src);
                        }
                    }
                }
            }
            let disk_path = std::fs::canonicalize(&dest).unwrap_or(dest);
            rel_path = format!("{}{}", REPO_MATERIAL_PREFIX, disk_path.to_string_lossy());
            stored_name = safe_name;
        }
    }

    sqlx::query("UPDATE material_files SET folder_id = ?, stored_name = ?, rel_path = ?, updated_at = ? WHERE id = ?")
        .bind(&folder_id)
        .bind(&stored_name)
        .bind(&rel_path)
        .bind(now_str())
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_material_file_meta(
    id: String,
    description: Option<String>,
    tags: Option<String>,
    state: State<'_, AppState>,
) -> Result<MaterialFile, String> {
    sqlx::query(
        "UPDATE material_files
         SET description = COALESCE(?, description),
             tags = COALESCE(?, tags),
             updated_at = ?
         WHERE id = ?",
    )
    .bind(&description)
    .bind(&tags)
    .bind(now_str())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_material_file(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let file = sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(f) = file {
        // 移入系统回收站（repo 与非 repo 物料统一处理）。repo 物料若仅删 DB 记录而留盘，
        // 下次同步会被重新扫入；移入回收站才能真正从 .autoforge/docs 移除且可恢复。
        let full_path = material_disk_path(&f);
        if full_path.exists() {
            move_path_to_trash(&full_path)?;
        }
        sqlx::query("DELETE FROM material_files WHERE id = ?")
            .bind(&id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn open_material_file(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let file = sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let full_path = material_disk_path(&file);
    if !full_path.exists() {
        return Err("文件不存在或已被移除".into());
    }

    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    cmd.arg(&full_path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开文件：{}", e))
}

#[tauri::command]
pub async fn material_file_data_url(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let file = sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // Cap inline previews: base64-encoding a large file into a data URL would
    // balloon memory and freeze the webview.
    const MAX_DATA_URL_BYTES: i64 = 16 * 1024 * 1024; // 16 MB
    if file.size_bytes > MAX_DATA_URL_BYTES {
        return Err(format!(
            "文件超过 {} MB，无法内联预览",
            MAX_DATA_URL_BYTES / 1024 / 1024
        ));
    }

    let full_path = material_disk_path(&file);
    let meta = std::fs::metadata(&full_path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_DATA_URL_BYTES as u64 {
        return Err("文件超过内联预览大小上限".to_string());
    }
    let bytes = std::fs::read(&full_path).map_err(|e| e.to_string())?;
    let b64 = general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", file.mime, b64))
}

// ── AI Organize ───────────────────────────────────────────────────────────────

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/yaml"
                | "application/toml"
                | "application/xml"
                | "application/javascript"
        )
}

#[tauri::command]
pub async fn ai_organize_materials(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let files = sqlx::query_as::<_, MaterialFile>(
        "SELECT * FROM material_files WHERE project_id = ? ORDER BY original_name",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if files.is_empty() {
        return Ok("暂无文件可整理".into());
    }

    let existing_folders = sqlx::query_as::<_, MaterialFolder>(
        "SELECT * FROM material_folders WHERE project_id = ? ORDER BY sort_order, name",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let existing_folder_map: std::collections::HashMap<String, MaterialFolder> =
        existing_folders
            .iter()
            .map(|f| (f.id.clone(), f.clone()))
            .collect();
    let existing_folder_labels: std::collections::HashSet<String> = existing_folders
        .iter()
        .map(|f| folder_path_label(Some(&f.id), &existing_folder_map))
        .collect();
    let mut existing_folder_list = existing_folder_labels
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    existing_folder_list.sort();
    let existing_folder_text = if existing_folder_list.is_empty() {
        "无".to_string()
    } else {
        existing_folder_list
            .iter()
            .map(|name| format!("- {}：用户已有文件夹", name))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build per-file entries that include a content snippet for text files.
    const SNIPPET_CHARS: usize = 1500;
    let mut file_list: Vec<String> = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        let header = format!(
            "[{}] {} ({} bytes, {})",
            i, f.original_name, f.size_bytes, f.mime
        );
        let body = if is_text_mime(&f.mime) {
            let disk_path = material_disk_path(f);
            match std::fs::read_to_string(&disk_path) {
                Ok(content) => {
                    let snippet: String = content.chars().take(SNIPPET_CHARS).collect();
                    let truncated = if content.chars().count() > SNIPPET_CHARS {
                        "…(截断)"
                    } else {
                        ""
                    };
                    format!("\n内容预览:\n{}{}", snippet, truncated)
                }
                Err(_) => "\n[文件内容不可读]".to_string(),
            }
        } else {
            "\n[二进制文件，仅凭文件名分类]".to_string()
        };
        file_list.push(format!("{}{}", header, body));
    }

    let fixed_folders = fixed_material_folder_list();
    let prompt = format!(
        r#"你是文档整理助手。仔细阅读以下每个文件的内容预览，根据文件的实际用途和内容把它们归入产品物料分类。

固定分类：
{}

用户已有文件夹（也可以作为 folder_name 使用，必须原样输出完整路径标签）：
{}

文件列表（共 {} 个）：

{}

严格按以下 JSON 格式输出，不要输出任何其他内容：
{{
  "assignments": [
    {{"file_index": 0, "folder_name": "产品需求"}}
  ],
  "summary": "整理说明（不超过80字）"
}}

要求：
- 必须为上面列出的全部 {} 个文件各分配一条 assignment，file_index 从 0 到 {}
- 优先根据文件内容和实际用途分类，而非文件扩展名
- 每个文件有且只有一个 folder_name，必须来自固定分类或用户已有文件夹
- 无法明确归类时使用 "其他"
- 不允许创造新的 folder_name；用户已有文件夹必须完整照抄上方路径标签
- assignments 数组长度必须等于文件总数 {}"#,
        fixed_folders,
        existing_folder_text,
        files.len(),
        file_list.join("\n\n---\n\n"),
        files.len(),
        files.len().saturating_sub(1),
        files.len(),
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        MATERIAL_AI_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的物料库 AI 助手，负责基于文件元数据和内容片段完成语义检索与整理。只输出调用方要求的格式。"),
        Some(&project_id),
        None, // 整理任务的 prompt 已是文件清单内容，直接作召回键即可
    )
        .await
        .map_err(|e| format!("AI 整理失败: {}", e))?;

    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回格式错误，JSON 不完整")?;
    let plan: OrgPlan =
        serde_json::from_str(&raw[start..=end]).map_err(|e| format!("解析 AI 输出失败: {}", e))?;

    let now = now_str();
    let mut assigned_folders = vec![OTHER_MATERIAL_FOLDER.to_string(); files.len()];
    for assignment in &plan.assignments {
        if assignment.file_index < files.len() {
            assigned_folders[assignment.file_index] = normalize_material_folder_name(
                &assignment.folder_name,
                &existing_folder_labels,
            );
        }
    }

    // Map folder name → (db_id, disk_path)
    // We must create directories on disk so sync_repo_docs can see them later.
    let mut folder_info: std::collections::HashMap<String, (String, Option<PathBuf>)> =
        Default::default();
    let mut used_folders = assigned_folders.clone();
    used_folders.sort_unstable();
    used_folders.dedup();
    let existing_folder_by_label: std::collections::HashMap<String, MaterialFolder> =
        existing_folders
            .into_iter()
            .map(|f| (folder_path_label(Some(&f.id), &existing_folder_map), f))
            .collect();

    for folder_name in used_folders {
        let folder = if let Some(existing) = existing_folder_by_label.get(&folder_name) {
            existing.clone()
        } else {
            get_or_create_material_folder(&project_id, None, &folder_name, &state).await?
        };

        // Create the corresponding disk directory so it survives the next sync.
        let disk_path = folder_path_from_docs(&project_id, Some(&folder.id), &state).await?;
        if let Some(ref dir) = disk_path {
            if let Err(e) = std::fs::create_dir_all(dir) {
                warn!(
                    "[ai_organize] failed to create dir {}: {}",
                    dir.display(),
                    e
                );
            }
        }

        folder_info.insert(folder_name, (folder.id, disk_path));
    }

    // Move each file to its assigned folder on disk AND update the DB.
    let mut moved = 0usize;
    for (file_index, folder_name) in assigned_folders.iter().enumerate() {
        let Some((folder_id, disk_dir)) = folder_info.get(folder_name.as_str()) else {
            continue;
        };

        let file = &files[file_index];
        let src = material_disk_path(file);

        let (new_rel_path, new_stored_name) = if let Some(dir) = disk_dir {
            if src.exists() {
                let dest = unique_child_path(dir, &file.original_name);
                if src != dest {
                    match std::fs::rename(&src, &dest) {
                        Ok(_) => {}
                        Err(_) => {
                            if let Err(e) = std::fs::copy(&src, &dest) {
                                warn!("[ai_organize] copy failed {}: {}", src.display(), e);
                                continue;
                            }
                            if is_repo_material(file) {
                                let _ = std::fs::remove_file(&src);
                            }
                        }
                    }
                }
                let canonical = std::fs::canonicalize(&dest).unwrap_or(dest.clone());
                let sname = dest
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or(&file.stored_name)
                    .to_string();
                (
                    format!("{}{}", REPO_MATERIAL_PREFIX, canonical.to_string_lossy()),
                    sname,
                )
            } else {
                (file.rel_path.clone(), file.stored_name.clone())
            }
        } else {
            (file.rel_path.clone(), file.stored_name.clone())
        };

        sqlx::query(
            "UPDATE material_files SET folder_id = ?, rel_path = ?, stored_name = ?, updated_at = ? WHERE id = ?",
        )
        .bind(folder_id)
        .bind(&new_rel_path)
        .bind(&new_stored_name)
        .bind(&now)
        .bind(&file.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        moved += 1;
    }

    Ok(format!("已整理 {} 个文件 · {}", moved, plan.summary))
}

// ── Backup ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_material_backup_config(
    state: State<'_, AppState>,
) -> Result<MaterialBackupConfig, String> {
    sqlx::query_as::<_, MaterialBackupConfig>(
        "SELECT * FROM material_backup_config WHERE id = 'singleton'",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_material_backup_config(
    provider: String,
    config_json: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<MaterialBackupConfig, String> {
    let _: serde_json::Value =
        serde_json::from_str(&config_json).map_err(|_| "config_json 不是有效的 JSON")?;

    sqlx::query(
        "UPDATE material_backup_config
         SET provider = ?, config_json = ?, enabled = ?, updated_at = ?
         WHERE id = 'singleton'",
    )
    .bind(&provider)
    .bind(&config_json)
    .bind(enabled as i32)
    .bind(now_str())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MaterialBackupConfig>(
        "SELECT * FROM material_backup_config WHERE id = 'singleton'",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn backup_material_files(
    project_id: String,
    file_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let config = sqlx::query_as::<_, MaterialBackupConfig>(
        "SELECT * FROM material_backup_config WHERE id = 'singleton'",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if !config.enabled || config.provider == "none" {
        return Err("备份未启用，请先在备份配置中设置存储提供商".into());
    }

    let files: Vec<MaterialFile> = if file_ids.is_empty() {
        sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE project_id = ?")
            .bind(&project_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let mut result = Vec::new();
        for fid in &file_ids {
            if let Ok(Some(f)) =
                sqlx::query_as::<_, MaterialFile>("SELECT * FROM material_files WHERE id = ?")
                    .bind(fid)
                    .fetch_optional(&state.db)
                    .await
            {
                result.push(f);
            }
        }
        result
    };

    let cfg_val: serde_json::Value = serde_json::from_str(&config.config_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let now = now_str();
    let mut success = 0usize;
    let mut errors = 0usize;

    for file in &files {
        let full_path = material_disk_path(file);
        if !full_path.exists() {
            warn!("[backup] file not on disk: {}", full_path.display());
            errors += 1;
            continue;
        }

        let result: Result<String, String> = match config.provider.as_str() {
            "local" => {
                // 未配置 path 时的兜底：走系统临时目录而非硬编码 /tmp（Windows 无 /tmp）。
                let default_dir = std::env::temp_dir().join("autoforge-backup");
                let dest_dir = cfg_val
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or(default_dir);
                let dest = dest_dir
                    .join(&project_id)
                    .join(&file.original_name);
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p).ok();
                }
                std::fs::copy(&full_path, &dest)
                    .map(|_| dest.to_string_lossy().to_string())
                    .map_err(|e| e.to_string())
            }
            "rclone" => {
                let remote = cfg_val
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .unwrap_or("remote");
                let path = cfg_val
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("AutoForge/materials");
                let dest = format!("{}:{}/{}", remote, path, project_id);
                let dest_clone = dest.clone();
                match tokio::process::Command::new("rclone")
                    .arg("copy")
                    .arg(&full_path)
                    .arg(&dest)
                    .output()
                    .await
                {
                    Ok(o) if o.status.success() => Ok(dest_clone),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
            p => Err(format!("不支持的备份提供商: {}", p)),
        };

        let (status, url) = match result {
            Ok(dest_url) => {
                success += 1;
                ("synced".to_string(), Some(dest_url))
            }
            Err(e) => {
                warn!("[backup] {} failed: {}", file.id, e);
                errors += 1;
                ("error".to_string(), None)
            }
        };

        let _ = sqlx::query(
            "UPDATE material_files
             SET backup_status = ?, backup_url = ?, backed_up_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&status)
        .bind(&url)
        .bind(if status == "synced" {
            Some(now.clone())
        } else {
            None
        })
        .bind(&now)
        .bind(&file.id)
        .execute(&state.db)
        .await;
    }

    Ok(format!("备份完成：成功 {} 个，失败 {} 个", success, errors))
}
