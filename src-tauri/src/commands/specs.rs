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
];
const SPEC_AI_SYSTEM_KIND: &str = "spec_writer";

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
        "SELECT * FROM project_specs WHERE project_id = ? AND category = ?
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
        buf.push_str(&format!("## {}\n\n{}\n", s.title, s.content));
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

    // Build @import lines only for spec files that actually exist
    let imports: String = CATEGORIES
        .iter()
        .filter(|(cat, _)| specs_dir.join(format!("{}.md", cat)).exists())
        .map(|(cat, _)| format!("@.autoforge/specs/{}.md\n", cat))
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

#[tauri::command]
pub async fn upsert_project_spec(
    project_id: String,
    id: Option<String>,
    category: String,
    title: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<ProjectSpec, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("规格标题不能为空".into());
    }
    if !CATEGORIES.iter().any(|(k, _)| *k == category) {
        return Err(format!("无效的规格分类: {}", category));
    }

    let now = now_str();

    if let Some(id) = id.filter(|s| !s.is_empty()) {
        sqlx::query(
            "UPDATE project_specs
             SET title = ?, content = ?, updated_at = ?
             WHERE id = ? AND project_id = ?",
        )
        .bind(&title)
        .bind(&content)
        .bind(&now)
        .bind(&id)
        .bind(&project_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        let spec = sqlx::query_as::<_, ProjectSpec>(
            "SELECT * FROM project_specs WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        write_category_file(&project_id, &category, &state).await?;
        return Ok(spec);
    }

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
         (id, project_id, category, title, content, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&category)
    .bind(&title)
    .bind(&content)
    .bind(next_order)
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

    write_category_file(&spec.project_id, &spec.category, &state).await?;
    Ok(true)
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
