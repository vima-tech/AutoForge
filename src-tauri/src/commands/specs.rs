use crate::models::project::Project;
use crate::models::spec::ProjectSpec;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::State;
use tracing::warn;
use uuid::Uuid;

const CATEGORIES: &[(&str, &str)] = &[
    ("tech_stack", "技术栈"),
    ("architecture", "架构约束"),
    ("coding", "编码规范"),
    ("api", "API 契约"),
    ("testing", "测试要求"),
    ("reference", "参考"),
];
/// 会聚合写出 `.autoforge/specs/{cat}.md` 并被 CLAUDE.md @import 的「宪法级」分类。
/// `reference` 分类（多为 agent 写的自由文件）不参与聚合写出与 @import。
const AGGREGATE_CATEGORIES: &[&str] = &["tech_stack", "architecture", "coding", "api", "testing"];
fn is_aggregate(category: &str) -> bool {
    AGGREGATE_CATEGORIES.contains(&category)
}
const SPEC_AI_SYSTEM_KIND: &str = "spec_writer";

/// 取项目仓库根路径（去空白；不校验存在性，调用方按需处理）。
async fn project_repo_path(project_id: &str, state: &AppState) -> Result<String, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT repo_path FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    Ok(row.map(|(p,)| p.trim().to_string()).unwrap_or_default())
}

/// 把仓库相对的 .autoforge/specs 文件路径规整为「相对 .autoforge/」形式（如 `specs/foo.md`），
/// 供 workspace 守卫复用。接受带或不带 `.autoforge/` 前缀的输入。
fn to_workspace_rel(rel_path: &str) -> String {
    let p = rel_path.trim().trim_start_matches("./");
    p.strip_prefix(".autoforge/").unwrap_or(p).to_string()
}

fn now_str() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn category_display(category: &str) -> &str {
    CATEGORIES
        .iter()
        .find(|(k, _)| *k == category)
        .map(|(_, v)| *v)
        .unwrap_or(category)
}

fn autoforge_specs_dir(repo_path: &str) -> Option<PathBuf> {
    let repo = PathBuf::from(repo_path.trim());
    if repo.is_dir() {
        Some(repo.join(".autoforge").join("specs"))
    } else {
        None
    }
}

async fn write_category_file(
    project_id: &str,
    category: &str,
    state: &AppState,
) -> Result<(), String> {
    // 仅宪法级分类聚合写出到 .autoforge/specs/{cat}.md；reference 等不落聚合文件。
    if !is_aggregate(category) {
        return Ok(());
    }
    let Some((repo_path,)) =
        sqlx::query_as::<_, (String,)>("SELECT repo_path FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };

    let Some(dir) = autoforge_specs_dir(&repo_path) else {
        return Ok(());
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("[specs] cannot create .autoforge/specs: {}", e);
        return Ok(());
    }

    let specs: Vec<ProjectSpec> = sqlx::query_as::<_, ProjectSpec>(
        "SELECT * FROM project_specs WHERE project_id = ? AND category = ? AND source = 'db'
         ORDER BY sort_order, created_at",
    )
    .bind(project_id)
    .bind(category)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let display = category_display(category);
    let mut buf = format!("# {}\n\n", display);
    for (i, s) in specs.iter().enumerate() {
        if i > 0 {
            buf.push_str("\n---\n\n");
        }
        // 嵌入不可见锚（HTML 注释），让聚合文件可被 parse_aggregate_file 反向解析回
        // 多条 db 源规格，保留 injection/order；AI 读取时锚为透明注释，无副作用。
        buf.push_str(&format!(
            "## {}\n<!-- autoforge:spec injection={} order={} -->\n\n{}\n",
            s.title, s.injection, s.sort_order, s.content
        ));
    }

    let file = dir.join(format!("{}.md", category));
    if let Err(e) = std::fs::write(&file, &buf) {
        warn!("[specs] failed to write {}: {}", file.display(), e);
    }

    update_claude_md_spec_section(&repo_path);
    Ok(())
}

const CLAUDE_MD_START: &str = "<!-- autoforge:specs:start -->";
const CLAUDE_MD_END:   &str = "<!-- autoforge:specs:end -->";

fn update_claude_md_spec_section(repo_path: &str) {
    let repo = std::path::Path::new(repo_path.trim());
    if !repo.is_dir() {
        return;
    }

    let specs_dir = repo.join(".autoforge").join("specs");

    // Build @import lines only for aggregate spec files that actually exist
    let imports: String = AGGREGATE_CATEGORIES
        .iter()
        .filter(|cat| specs_dir.join(format!("{}.md", cat)).exists())
        .map(|cat| format!("@.autoforge/specs/{}.md\n", cat))
        .collect();

    if imports.is_empty() {
        return;
    }

    let section = format!(
        "{start}\n## AutoForge 项目规格\n\n\
         以下为 AutoForge 管理的项目规格约束，AI 执行任务时必须遵守：\n\n\
         {imports}\n{end}",
        start = CLAUDE_MD_START,
        imports = imports.trim_end(),
        end = CLAUDE_MD_END,
    );

    let claude_md = repo.join("CLAUDE.md");
    let existing = std::fs::read_to_string(&claude_md).unwrap_or_default();

    let new_content = if let (Some(s), Some(e)) = (
        existing.find(CLAUDE_MD_START),
        existing.find(CLAUDE_MD_END),
    ) {
        // Replace the existing AutoForge section in-place
        let end_pos = e + CLAUDE_MD_END.len();
        format!("{}{}{}", &existing[..s], section, &existing[end_pos..])
    } else if existing.is_empty() {
        section
    } else {
        format!("{}\n\n{}\n", existing.trim_end(), section)
    };

    if let Err(e) = std::fs::write(&claude_md, new_content) {
        warn!("[specs] failed to update CLAUDE.md: {}", e);
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_project_specs(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectSpec>, String> {
    sqlx::query_as::<_, ProjectSpec>(
        "SELECT * FROM project_specs WHERE project_id = ?
         ORDER BY category, sort_order, created_at",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

fn normalize_injection(mode: &str) -> Result<String, String> {
    match mode {
        "always" | "on_demand" | "off" => Ok(mode.to_string()),
        _ => Err(format!("无效的注入档位: {}", mode)),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn upsert_project_spec(
    project_id: String,
    id: Option<String>,
    category: String,
    title: String,
    content: String,
    description: Option<String>,
    injection: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectSpec, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("规格标题不能为空".into());
    }
    if !CATEGORIES.iter().any(|(k, _)| *k == category) {
        return Err(format!("无效的规格分类: {}", category));
    }
    let description = description.unwrap_or_default();
    let injection = normalize_injection(injection.as_deref().unwrap_or("always"))?;

    let now = now_str();

    if let Some(id) = id.filter(|s| !s.is_empty()) {
        // 取现有行以判断来源（file 源写回磁盘，content 不入库）。
        let existing = sqlx::query_as::<_, ProjectSpec>(
            "SELECT * FROM project_specs WHERE id = ? AND project_id = ?",
        )
        .bind(&id)
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("规格不存在")?;

        if existing.source == "file" {
            // 内容写回其文件，DB content 保持空。
            let repo_path = project_repo_path(&project_id, &state).await?;
            if repo_path.is_empty() {
                return Err("项目未设置仓库路径".into());
            }
            let rel = to_workspace_rel(&existing.rel_path);
            crate::commands::workspace::write_workspace_path(&repo_path, &rel, &content).await?;
            sqlx::query(
                "UPDATE project_specs
                 SET title = ?, category = ?, description = ?, injection = ?, updated_at = ?
                 WHERE id = ? AND project_id = ?",
            )
            .bind(&title)
            .bind(&category)
            .bind(&description)
            .bind(&injection)
            .bind(&now)
            .bind(&id)
            .bind(&project_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        } else {
            sqlx::query(
                "UPDATE project_specs
                 SET title = ?, category = ?, content = ?, description = ?, injection = ?, updated_at = ?
                 WHERE id = ? AND project_id = ?",
            )
            .bind(&title)
            .bind(&category)
            .bind(&content)
            .bind(&description)
            .bind(&injection)
            .bind(&now)
            .bind(&id)
            .bind(&project_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        }

        let spec = sqlx::query_as::<_, ProjectSpec>("SELECT * FROM project_specs WHERE id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        // 分类可能变动：旧分类与新分类的聚合文件都重写（仅 db 源 + 宪法级分类生效）。
        if existing.category != category {
            write_category_file(&project_id, &existing.category, &state).await?;
        }
        write_category_file(&project_id, &category, &state).await?;
        return Ok(spec);
    }

    // 新建：UI 只创建 db 源规格（file 源由扫描登记）。
    let (next_order,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(sort_order), -1) + 1
         FROM project_specs WHERE project_id = ? AND category = ?",
    )
    .bind(&project_id)
    .bind(&category)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO project_specs
         (id, project_id, category, title, content, sort_order, source, rel_path, description, injection, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'db', '', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&category)
    .bind(&title)
    .bind(&content)
    .bind(next_order)
    .bind(&description)
    .bind(&injection)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let spec = sqlx::query_as::<_, ProjectSpec>("SELECT * FROM project_specs WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    write_category_file(&project_id, &category, &state).await?;
    Ok(spec)
}

#[tauri::command]
pub async fn delete_project_spec(
    id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let Some(spec) =
        sqlx::query_as::<_, ProjectSpec>("SELECT * FROM project_specs WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };

    sqlx::query("DELETE FROM project_specs WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // file 源连带删除磁盘文件（经工作区守卫，越界/失败仅告警不阻断索引删除）。
    if spec.source == "file" && !spec.rel_path.is_empty() {
        let repo_path = project_repo_path(&spec.project_id, &state).await?;
        if !repo_path.is_empty() {
            let rel = to_workspace_rel(&spec.rel_path);
            if let Err(e) = crate::commands::workspace::delete_workspace_path(&repo_path, &rel).await {
                warn!("[specs] 删除文件 {} 失败: {}", spec.rel_path, e);
            }
        }
    }

    write_category_file(&spec.project_id, &spec.category, &state).await?;
    Ok(true)
}

// ── 文件规格对账 / 读取 / 注入档位 ─────────────────────────────────────────────

/// 取文件首个非空、非标题行作为描述（截断 120 字符）；退化用首个标题行。
fn derive_description(content: &str) -> String {
    let mut heading: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            if heading.is_none() {
                heading = Some(h.trim_start_matches('#').trim().to_string());
            }
            continue;
        }
        if t.starts_with("> ") || t == ">" {
            continue;
        }
        return t.chars().take(120).collect();
    }
    heading.unwrap_or_default().chars().take(120).collect()
}

/// 聚合文件解析出的一条规格（标题 / 正文 / 注入档位）。
struct ParsedSpec {
    title: String,
    content: String,
    injection: String,
}

/// 把聚合产物文件（`# 显示名` + 多个 `## 标题` 段，段间 `---` 分隔）解析回多条规格。
/// 容忍带或不带 `<!-- autoforge:spec injection=.. order=.. -->` 锚：有锚则取其 injection，
/// 无锚（旧文件/手写）默认 'always'。供跨机器/清库后从磁盘恢复 db 源规格。
fn parse_aggregate_file(text: &str) -> Vec<ParsedSpec> {
    let mut out: Vec<ParsedSpec> = Vec::new();
    let mut title: Option<String> = None;
    let mut injection = String::from("always");
    let mut buf: Vec<&str> = Vec::new();

    let flush = |out: &mut Vec<ParsedSpec>, title: &mut Option<String>, injection: &mut String, buf: &mut Vec<&str>| {
        if let Some(t) = title.take() {
            let content = buf.join("\n").trim_matches(|c| c == '\n' || c == '\r').to_string();
            let inj = normalize_injection(injection).unwrap_or_else(|_| "always".to_string());
            out.push(ParsedSpec { title: t.trim().to_string(), content, injection: inj });
        }
        buf.clear();
        *injection = String::from("always");
    };

    for line in text.lines() {
        let t = line.trim();
        if t == "---" {
            flush(&mut out, &mut title, &mut injection, &mut buf);
            continue;
        }
        if let Some(h) = t.strip_prefix("## ") {
            flush(&mut out, &mut title, &mut injection, &mut buf);
            title = Some(h.to_string());
            continue;
        }
        // 顶层文档标题（`# 显示名`）在尚未进入任何规格段时跳过。
        if title.is_none() && t.starts_with("# ") {
            continue;
        }
        if let Some(rest) = t.strip_prefix("<!-- autoforge:spec").and_then(|s| s.strip_suffix("-->")) {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("injection=") {
                    injection = v.to_string();
                }
            }
            continue;
        }
        if title.is_some() {
            buf.push(line);
        }
    }
    flush(&mut out, &mut title, &mut injection, &mut buf);
    out
}

/// A) 解析 5 个聚合文件，把缺失的 db 源规格按标题回灌（跨机器/清库后恢复 AI 生成的规格）。
/// 幂等：仅插入 DB 中尚不存在的标题。返回恢复条数。
async fn recover_db_specs_from_disk(
    project_id: &str,
    specs_dir: &std::path::Path,
    state: &AppState,
) -> Result<usize, String> {
    let now = now_str();
    let mut recovered = 0usize;
    for cat in AGGREGATE_CATEGORIES {
        let file = specs_dir.join(format!("{}.md", cat));
        let Ok(text) = std::fs::read_to_string(&file) else { continue; };
        let parsed = parse_aggregate_file(&text);
        if parsed.is_empty() {
            continue;
        }
        let existing_titles: Vec<(String,)> = sqlx::query_as(
            "SELECT title FROM project_specs WHERE project_id = ? AND category = ? AND source = 'db'",
        )
        .bind(project_id)
        .bind(cat)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        let have: std::collections::HashSet<String> =
            existing_titles.into_iter().map(|(t,)| t).collect();
        let (mut order,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM project_specs WHERE project_id = ? AND category = ?",
        )
        .bind(project_id)
        .bind(cat)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        for item in parsed {
            if item.title.is_empty() || have.contains(&item.title) {
                continue;
            }
            let id = Uuid::new_v4().to_string();
            let description = derive_description(&item.content);
            sqlx::query(
                "INSERT INTO project_specs
                 (id, project_id, category, title, content, sort_order, source, rel_path, description, injection, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'db', '', ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(project_id)
            .bind(cat)
            .bind(&item.title)
            .bind(&item.content)
            .bind(order)
            .bind(&description)
            .bind(&item.injection)
            .bind(&now)
            .bind(&now)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            order += 1;
            recovered += 1;
        }
    }
    Ok(recovered)
}

/// B) 自由 .md 文件 → source='file' reference 对账。返回 (新登记, 清理)。
async fn scan_reference_files(
    project_id: &str,
    specs_dir: &std::path::Path,
    state: &AppState,
) -> Result<(usize, usize), String> {
    // 现有 file 源行：rel_path -> id（用于去重与清理）。
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, rel_path FROM project_specs WHERE project_id = ? AND source = 'file'",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let aggregate_names: Vec<String> = AGGREGATE_CATEGORIES
        .iter()
        .map(|c| format!("{}.md", c))
        .collect();

    // 1) 扫描磁盘 .md 文件，收集自由文件的 workspace 相对路径。
    let mut on_disk: Vec<(String, String, String)> = Vec::new(); // (rel `specs/x.md`, file_name, full_path)
    if specs_dir.is_dir() {
        let mut rd = tokio::fs::read_dir(specs_dir).await.map_err(|e| e.to_string())?;
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(fname) = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else {
                continue;
            };
            if !fname.to_lowercase().ends_with(".md") {
                continue;
            }
            if aggregate_names.iter().any(|a| a == &fname) {
                continue; // 聚合产物文件不登记
            }
            on_disk.push((format!("specs/{}", fname), fname, path.to_string_lossy().to_string()));
        }
    }

    let existing_rels: std::collections::HashSet<String> =
        existing.iter().map(|(_, r)| to_workspace_rel(r)).collect();

    let now = now_str();
    let mut added = 0usize;

    // 2) 新文件 → 登记。
    let (next_order,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(sort_order), -1) + 1
         FROM project_specs WHERE project_id = ? AND category = 'reference'",
    )
    .bind(project_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let mut order = next_order;

    for (rel, fname, full) in &on_disk {
        if existing_rels.contains(rel) {
            continue;
        }
        let content = tokio::fs::read_to_string(full).await.unwrap_or_default();
        let description = derive_description(&content);
        let title = fname.trim_end_matches(".md").to_string();
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO project_specs
             (id, project_id, category, title, content, sort_order, source, rel_path, description, injection, created_at, updated_at)
             VALUES (?, ?, 'reference', ?, '', ?, 'file', ?, ?, 'on_demand', ?, ?)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(&title)
        .bind(order)
        .bind(rel)
        .bind(&description)
        .bind(&now)
        .bind(&now)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        order += 1;
        added += 1;
    }

    // 3) 文件已消失的 file 源行 → 清理。
    let disk_rels: std::collections::HashSet<String> =
        on_disk.iter().map(|(r, _, _)| r.clone()).collect();
    let mut removed = 0usize;
    for (rid, rrel) in &existing {
        if !disk_rels.contains(&to_workspace_rel(rrel)) {
            sqlx::query("DELETE FROM project_specs WHERE id = ?")
                .bind(rid)
                .execute(&state.db)
                .await
                .map_err(|e| e.to_string())?;
            removed += 1;
        }
    }

    Ok((added, removed))
}

/// 把磁盘 `.autoforge/specs/` 反向对账回 DB（A 聚合 db 源 + B 自由 file 源），幂等。
/// 由「重新扫描」命令与项目创建/重挂时调用，是规格内容跨机器/清库恢复的唯一入口。
pub async fn reconcile_specs_from_disk(
    project_id: &str,
    state: &AppState,
) -> Result<String, String> {
    let repo_path = project_repo_path(project_id, state).await?;
    if repo_path.is_empty() {
        return Err("项目未设置仓库路径".into());
    }
    let specs_dir = match autoforge_specs_dir(&repo_path) {
        Some(d) => d,
        None => return Ok("项目目录不存在，未扫描".into()),
    };
    let recovered = recover_db_specs_from_disk(project_id, &specs_dir, state).await?;
    let (added, removed) = scan_reference_files(project_id, &specs_dir, state).await?;
    // 恢复了 db 源规格则刷新聚合文件 + CLAUDE.md @import，保持磁盘与 DB 一致。
    if recovered > 0 {
        for cat in AGGREGATE_CATEGORIES {
            write_category_file(project_id, cat, state).await?;
        }
    }
    Ok(format!(
        "对账完成：恢复 {} 条 DB 规格，新登记 {} 个文件规格，清理 {} 个失效条目",
        recovered, added, removed
    ))
}

/// 扫描 / 反向对账 `.autoforge/specs/*.md` 与索引（薄命令，委托 reconcile）。
#[tauri::command]
pub async fn scan_spec_files(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    reconcile_specs_from_disk(&project_id, &state).await
}

/// 取规格全文：db 源返回内联 content；file 源经守卫读盘。
#[tauri::command]
pub async fn get_spec_content(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let spec = sqlx::query_as::<_, ProjectSpec>("SELECT * FROM project_specs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("规格不存在")?;

    if spec.source == "file" {
        let repo_path = project_repo_path(&spec.project_id, &state).await?;
        if repo_path.is_empty() {
            return Err("项目未设置仓库路径".into());
        }
        let rel = to_workspace_rel(&spec.rel_path);
        crate::commands::workspace::read_workspace_path(&repo_path, &rel).await
    } else {
        Ok(spec.content)
    }
}

/// 轻量切换注入档位（不重写文件）。
#[tauri::command]
pub async fn set_spec_injection(
    id: String,
    injection: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let injection = normalize_injection(&injection)?;
    let res = sqlx::query("UPDATE project_specs SET injection = ?, updated_at = ? WHERE id = ?")
        .bind(&injection)
        .bind(now_str())
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.rows_affected() > 0)
}

// ── AI Generation ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct AiSpecItem {
    title: String,
    content: String,
}

#[derive(serde::Deserialize)]
struct AiSpecPlan {
    tech_stack: Vec<AiSpecItem>,
    architecture: Vec<AiSpecItem>,
    coding: Vec<AiSpecItem>,
    api: Vec<AiSpecItem>,
    testing: Vec<AiSpecItem>,
}

#[tauri::command]
pub async fn ai_generate_specs(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let Some(project) =
        sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Err("项目不存在".into());
    };

    // Scan key tech files from repo (first 800 chars each)
    let mut tech_files: Vec<String> = Vec::new();
    if !project.repo_path.is_empty() {
        let repo = std::path::Path::new(&project.repo_path);
        for name in &[
            "Cargo.toml",
            "package.json",
            "requirements.txt",
            "go.mod",
            "pom.xml",
            "README.md",
        ] {
            let p = repo.join(name);
            if p.exists() {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    let truncated: String = text.chars().take(800).collect();
                    tech_files.push(format!("=== {} ===\n{}", name, truncated));
                }
            }
        }
    }

    let materials: Vec<(String,)> =
        sqlx::query_as("SELECT original_name FROM material_files WHERE project_id = ? LIMIT 30")
            .bind(&project_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let mat_list = if materials.is_empty() {
        "无".to_string()
    } else {
        materials
            .iter()
            .map(|(n,)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let tech_summary = if tech_files.is_empty() {
        "无技术文件".to_string()
    } else {
        tech_files.join("\n\n")
    };

    let prompt = format!(
        r#"你是技术规格制定专家。根据以下项目信息，为项目生成结构化的规格约束条目。

项目名称：{name}
项目描述：{desc}
物料文档：{mat}

技术文件摘要：
{tech}

要求：
- 每个分类生成 2-5 条规格
- 每条规格的 content 简洁、可执行，用中文，不超过 150 字
- 根据实际技术文件推断具体版本、框架、约定

严格按以下 JSON 格式输出，不输出任何其他内容：
{{
  "tech_stack":    [{{"title": "...", "content": "..."}}],
  "architecture":  [{{"title": "...", "content": "..."}}],
  "coding":        [{{"title": "...", "content": "..."}}],
  "api":           [{{"title": "...", "content": "..."}}],
  "testing":       [{{"title": "...", "content": "..."}}]
}}"#,
        name = project.name,
        desc = project.description,
        mat = mat_list,
        tech = tech_summary,
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        SPEC_AI_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的项目规格生成 Agent，负责把项目信息、技术文件和物料摘要转换为可执行的结构化规格约束。只输出调用方要求的 JSON。"),
        Some(&project_id),
        None, // 规格生成的 prompt 已含项目技术上下文，作召回键足够
    )
        .await
        .map_err(|e| format!("AI 生成失败: {}", e))?;

    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回 JSON 不完整")?;
    let plan: AiSpecPlan = serde_json::from_str(&raw[start..=end])
        .map_err(|e| format!("解析 AI 输出失败: {}", e))?;

    // Replace all existing specs for this project
    sqlx::query("DELETE FROM project_specs WHERE project_id = ?")
        .bind(&project_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let now = now_str();
    let mut total = 0usize;

    let buckets: [(&str, &Vec<AiSpecItem>); 5] = [
        ("tech_stack", &plan.tech_stack),
        ("architecture", &plan.architecture),
        ("coding", &plan.coding),
        ("api", &plan.api),
        ("testing", &plan.testing),
    ];

    for (category, items) in &buckets {
        for (i, item) in items.iter().enumerate() {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO project_specs
                 (id, project_id, category, title, content, sort_order, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&project_id)
            .bind(category)
            .bind(&item.title)
            .bind(&item.content)
            .bind(i as i64)
            .bind(&now)
            .bind(&now)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            total += 1;
        }
        write_category_file(&project_id, category, &state).await?;
    }

    Ok(format!("已生成 {} 条规格，已写入 .autoforge/specs/", total))
}

/// 追加一批 db 源规格并刷新对应聚合文件 + CLAUDE.md @import。供项目蓝图 apply 复用
/// （与 ai_generate_specs 同一落库/落盘路径，但**追加**而非清空既有规格）。
/// 非宪法级 category 回落到 architecture，避免散落分类不被聚合写出。
/// 入参 `specs`：`(category, title, content)`。返回写入条数。
pub async fn insert_db_specs(
    project_id: &str,
    specs: &[(String, String, String)],
    state: &AppState,
) -> Result<usize, String> {
    let now = now_str();
    let mut touched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (category, title, content) in specs {
        let cat = if is_aggregate(category) {
            category.clone()
        } else {
            "architecture".to_string()
        };
        // 续接该分类现有最大 sort_order，保持稳定排序。
        let next: i64 = sqlx::query_as::<_, (Option<i64>,)>(
            "SELECT MAX(sort_order) FROM project_specs WHERE project_id = ? AND category = ?",
        )
        .bind(project_id)
        .bind(&cat)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .0
        .map(|v| v + 1)
        .unwrap_or(0);
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO project_specs
             (id, project_id, category, title, content, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(&cat)
        .bind(title)
        .bind(content)
        .bind(next)
        .bind(&now)
        .bind(&now)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        touched.insert(cat);
    }
    for cat in &touched {
        write_category_file(project_id, cat, state).await?;
    }
    Ok(specs.len())
}
