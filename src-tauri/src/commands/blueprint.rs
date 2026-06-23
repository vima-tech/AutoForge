//! 项目蓝图向导：把一段「大需求」一次性炼成 PRD + 技术规格 + TASKLIST。
//!
//! 三段式（均显式、人审优先）：
//! 1. `generate_project_blueprint`：调 spec_writer 角色 LLM，**只**返回结构化结果供前端预览，
//!    不写盘、不入池。
//! 2. `apply_project_blueprint`：把 PRD 写入 `.autoforge/docs/`、规格写入 `.autoforge/specs/`
//!    （经 workspace 守卫 + DB 登记）。
//! 3. `enqueue_blueprint_tasks`：把人工勾选的 TASKLIST 任务逐条入 triage 待整理池（人保留闸口）。

use crate::intake::{gateway, IntakeMode, IntakePayload};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

/// 蓝图里的一条技术规格。`category` 期望落在宪法级集合
/// （tech_stack/architecture/coding/api/testing），否则 apply 时回落 architecture。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintSpec {
    pub category: String,
    pub title: String,
    pub content_markdown: String,
}

/// 蓝图里的一条任务（→ 可一键转为 triage 需求）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintTask {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_category() -> String {
    "Feature".to_string()
}
fn default_severity() -> String {
    "medium".to_string()
}

/// AI 生成蓝图的完整结果（供前端预览 + 后续 apply/enqueue）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintResult {
    pub prd_markdown: String,
    pub specs: Vec<BlueprintSpec>,
    pub tasklist: Vec<BlueprintTask>,
    /// 由 tasklist 渲染的 Markdown 清单，供可选写入 docs/TASKLIST.md。
    pub tasklist_markdown: String,
}

/// LLM 原始 JSON 形状（容错：缺字段时各有默认）。
#[derive(Debug, Deserialize)]
struct RawBlueprint {
    #[serde(default)]
    prd_markdown: String,
    #[serde(default)]
    specs: Vec<BlueprintSpec>,
    #[serde(default)]
    tasklist: Vec<BlueprintTask>,
}

const BLUEPRINT_SYSTEM_KIND: &str = "spec_writer";

/// 步骤1：根据大需求生成蓝图（PRD/specs/tasklist），仅返回结构化结果，不落盘、不入池。
#[tauri::command]
pub async fn generate_project_blueprint(
    project_id: String,
    brief: String,
    state: State<'_, AppState>,
) -> Result<BlueprintResult, String> {
    let brief = brief.trim();
    if brief.is_empty() {
        return Err("大需求内容不能为空".into());
    }
    // 安全铁律：外部输入入 AI 前过注入过滤。
    if crate::core::security::has_obvious_injection(brief) {
        return Err("大需求文本疑似含注入内容，已拒绝".into());
    }

    let project = sqlx::query_as::<_, crate::models::project::Project>(
        "SELECT * FROM projects WHERE id = ?",
    )
    .bind(&project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or("项目不存在")?;

    // 复用需求分析的项目上下文构建（claude.md/agents.md/技术文件/目录树等），让蓝图贴合实际仓库。
    let project_ctx = if project.repo_path.trim().is_empty() {
        String::new()
    } else {
        crate::agents::analysis::build_project_context(&project.repo_path).await
    };
    let ctx_block = if project_ctx.trim().is_empty() {
        "（无既有仓库上下文，按全新项目处理）".to_string()
    } else {
        project_ctx
    };

    let prompt = format!(
        r#"你是资深产品 + 架构师。下面是一个项目的「大需求」原始描述，请把它炼成一套初始项目蓝图。

项目名称：{name}
项目描述：{desc}

既有仓库上下文：
{ctx}

大需求原文：
{brief}

请输出三部分：
1. prd_markdown：一份结构化的产品需求文档（PRD），Markdown 格式，含背景/目标/用户场景/功能范围/非目标/验收标准等小节。
2. specs：技术规格条目数组，每条含 category（只能取 tech_stack/architecture/coding/api/testing 之一）、title、content_markdown（该约束的简洁可执行说明）。
3. tasklist：可执行任务清单数组，每条含 title（简短动宾）、description（一两句交代要做什么）、category（Feature/Bug/Refactor/Chore 之一）、severity（low/medium/high）。任务应可被逐条独立实现。

严格只输出如下 JSON，不要任何额外文字或代码围栏：
{{
  "prd_markdown": "...",
  "specs": [{{"category":"architecture","title":"...","content_markdown":"..."}}],
  "tasklist": [{{"title":"...","description":"...","category":"Feature","severity":"medium"}}]
}}"#,
        name = project.name,
        desc = project.description,
        ctx = ctx_block,
        brief = brief,
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        BLUEPRINT_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的项目蓝图生成 Agent，把一段大需求拆解为 PRD、技术规格与可执行任务清单。只输出调用方要求的 JSON。"),
        Some(&project_id),
        Some(brief),
    )
    .await
    .map_err(|e| format!("AI 生成失败: {}", e))?;

    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回 JSON 不完整")?;
    let parsed: RawBlueprint = serde_json::from_str(&raw[start..=end])
        .map_err(|e| format!("解析 AI 输出失败: {}", e))?;

    if parsed.prd_markdown.trim().is_empty() && parsed.specs.is_empty() && parsed.tasklist.is_empty()
    {
        return Err("AI 未能从该大需求生成有效蓝图，请补充更具体的描述后重试".into());
    }

    let tasklist_markdown = render_tasklist_md(&parsed.tasklist);
    Ok(BlueprintResult {
        prd_markdown: parsed.prd_markdown,
        specs: parsed.specs,
        tasklist: parsed.tasklist,
        tasklist_markdown,
    })
}

fn render_tasklist_md(tasks: &[BlueprintTask]) -> String {
    if tasks.is_empty() {
        return "# 任务清单\n\n（无任务）\n".to_string();
    }
    let mut buf = String::from("# 任务清单\n\n");
    for t in tasks {
        buf.push_str(&format!(
            "- [ ] **{}** `{}`/`{}`\n",
            t.title.trim(),
            t.category.trim(),
            t.severity.trim()
        ));
        let desc = t.description.trim();
        if !desc.is_empty() {
            buf.push_str(&format!("  - {}\n", desc));
        }
    }
    buf
}

/// 步骤2：把蓝图落入 `.autoforge` 工作区——PRD/可选 TASKLIST 写 docs，规格写 specs 并登记 DB。
#[tauri::command]
pub async fn apply_project_blueprint(
    project_id: String,
    prd_markdown: String,
    specs: Vec<BlueprintSpec>,
    tasklist_markdown: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo_path: String =
        sqlx::query_as::<_, (String,)>("SELECT repo_path FROM projects WHERE id = ?")
            .bind(&project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .map(|(p,)| p.trim().to_string())
            .ok_or("项目不存在")?;
    if repo_path.is_empty() {
        return Err("项目未设置本地仓库路径，无法写入工作区".into());
    }

    let mut written: Vec<String> = Vec::new();

    if !prd_markdown.trim().is_empty() {
        crate::commands::workspace::write_workspace_path(&repo_path, "docs/PRD.md", &prd_markdown)
            .await?;
        written.push("docs/PRD.md".into());
    }

    if let Some(tl) = tasklist_markdown
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        crate::commands::workspace::write_workspace_path(&repo_path, "docs/TASKLIST.md", tl).await?;
        written.push("docs/TASKLIST.md".into());
    }

    let spec_tuples: Vec<(String, String, String)> = specs
        .into_iter()
        .filter(|s| !s.title.trim().is_empty() && !s.content_markdown.trim().is_empty())
        .map(|s| (s.category, s.title, s.content_markdown))
        .collect();
    let spec_count = if spec_tuples.is_empty() {
        0
    } else {
        crate::commands::specs::insert_db_specs(&project_id, &spec_tuples, &state).await?
    };

    Ok(format!(
        "已写入工作区文档 {} 个，登记规格 {} 条",
        written.len(),
        spec_count
    ))
}

/// 步骤3：把人工勾选的任务逐条入 triage 待整理池（人保留闸口）。返回成功入池条数。
/// 每条经 gateway::receive(mode=Triage)，与速录同款链路（含后续安全过滤）。
#[tauri::command]
pub async fn enqueue_blueprint_tasks(
    project_id: String,
    tasks: Vec<BlueprintTask>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let project_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if project_exists.is_none() {
        return Err("项目不存在".into());
    }

    let mut enqueued = 0usize;
    for t in tasks {
        let title = t.title.trim();
        if title.is_empty() {
            continue;
        }
        // 注入过滤：标题 + 描述任一疑似注入则跳过该条（不阻断其余）。
        if crate::core::security::has_obvious_injection(title)
            || crate::core::security::has_obvious_injection(&t.description)
        {
            continue;
        }
        let desc = if t.description.trim().is_empty() {
            title.to_string()
        } else {
            t.description.trim().to_string()
        };
        let payload = IntakePayload {
            project_id: project_id.clone(),
            title: title.to_string(),
            description: Some(desc),
            category: Some(t.category.clone()),
            severity: Some(t.severity.clone()),
            source_type: "blueprint".to_string(),
            source_ref: None,
        };
        if gateway::receive(&state.db, &state.job_tx, &app, payload, IntakeMode::Triage)
            .await
            .is_ok()
        {
            enqueued += 1;
        }
    }
    Ok(enqueued)
}
