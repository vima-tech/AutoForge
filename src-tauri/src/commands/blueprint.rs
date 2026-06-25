//! 项目蓝图 2.0：可持久、可多轮对话打磨的蓝图工作台后端。
//!
//! 草稿是**单一真源**（PRD + 规格 + 任务清单），AI 修正与人工手改都写同一份；
//! 满意后才落 `.autoforge/` 工作区 + 入 triage 池（人审闸口不变）。
//!
//! 命令一览（均薄包装：取 state → 调纯 async fn → 返回，事件不在此发）：
//! - `start_blueprint_draft`：大需求 → 起草并持久一份新草稿（清掉该项目旧草稿）。
//! - `refine_blueprint_draft`：自然语言指令 → AI 回传整份更新草稿（保留稳定 id），存库 + 记对话。
//! - `patch_blueprint_draft`：人工手改（PRD 段 / 规格 / 任务增删改）落库。
//! - `get_blueprint_draft`：恢复某项目当前草稿 + 对话历史。
//! - `apply_blueprint_draft`：定稿 = PRD 写 docs、规格写 specs+DB、勾选任务入 triage 池。

use crate::models::blueprint::{
    BlueprintDraft, BlueprintDraftSummary, BlueprintDraftView, BlueprintMessage, BlueprintSpec,
    BlueprintTask,
};
use crate::state::AppState;
use serde::Deserialize;
use tauri::{AppHandle, State};
use uuid::Uuid;

const BLUEPRINT_SYSTEM_KIND: &str = "spec_writer";
/// 多轮修正喂回模型的最多历史条数（更早的轮次省略，控制 token）。
const HISTORY_CAP: usize = 12;

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// LLM 起草/修正的原始 JSON 形状（容错：缺字段时各有默认；id 缺省由我们补）。
#[derive(Debug, Default, Deserialize)]
struct RawBlueprint {
    #[serde(default)]
    prd_markdown: String,
    #[serde(default)]
    specs: Vec<BlueprintSpec>,
    #[serde(default)]
    tasklist: Vec<BlueprintTask>,
    /// 仅修正轮有意义：这一轮改了什么的一句话摘要。
    #[serde(default)]
    change_summary: String,
}

/// 给规格/任务补齐稳定 id（模型未回传或起草新建时）。已有非空 id 原样保留。
fn ensure_ids(specs: &mut [BlueprintSpec], tasks: &mut [BlueprintTask]) {
    for s in specs.iter_mut() {
        if s.id.trim().is_empty() {
            s.id = Uuid::new_v4().to_string();
        }
    }
    for t in tasks.iter_mut() {
        if t.id.trim().is_empty() {
            t.id = Uuid::new_v4().to_string();
        }
    }
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

/// 从原始 LLM 文本中抠出第一个 `{` 到最后一个 `}` 并解析为 RawBlueprint。
fn parse_raw(raw: &str) -> Result<RawBlueprint, String> {
    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回 JSON 不完整")?;
    serde_json::from_str(&raw[start..=end]).map_err(|e| format!("解析 AI 输出失败: {}", e))
}

/// 把一行草稿记录（specs/tasklist 为 JSON 文本）组装成结构化 BlueprintDraft。
#[allow(clippy::too_many_arguments)]
fn row_to_draft(
    id: String,
    project_id: String,
    title: String,
    brief: String,
    prd_markdown: String,
    specs_json: String,
    tasklist_json: String,
    status: String,
    issue_id: String,
    cr_id: String,
    created_at: String,
    updated_at: String,
) -> BlueprintDraft {
    let specs: Vec<BlueprintSpec> = serde_json::from_str(&specs_json).unwrap_or_default();
    let tasklist: Vec<BlueprintTask> = serde_json::from_str(&tasklist_json).unwrap_or_default();
    BlueprintDraft {
        id,
        project_id,
        title,
        brief,
        prd_markdown,
        specs,
        tasklist,
        status,
        issue_id,
        cr_id,
        created_at,
        updated_at,
    }
}

type DraftRow = (
    String, String, String, String, String, String,
    String, String, String, String, String, String,
);

async fn fetch_draft(db: &crate::db::Db, draft_id: &str) -> Result<BlueprintDraft, String> {
    let row: Option<DraftRow> = sqlx::query_as(
        "SELECT id, project_id, title, brief, prd_markdown, specs_json, tasklist_json, status, issue_id, cr_id, created_at, updated_at
         FROM blueprint_drafts WHERE id = ?",
    )
    .bind(draft_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    let (id, project_id, title, brief, prd, specs_json, tasklist_json, status, issue_id, cr_id, created, updated) =
        row.ok_or("蓝图草稿不存在")?;
    Ok(row_to_draft(
        id, project_id, title, brief, prd, specs_json, tasklist_json, status, issue_id, cr_id, created, updated,
    ))
}

async fn fetch_messages(
    db: &crate::db::Db,
    draft_id: &str,
) -> Result<Vec<BlueprintMessage>, String> {
    sqlx::query_as::<_, BlueprintMessage>(
        "SELECT id, draft_id, role, content, change_summary, created_at
         FROM blueprint_messages WHERE draft_id = ? ORDER BY created_at",
    )
    .bind(draft_id)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())
}

async fn load_view(db: &crate::db::Db, draft_id: &str) -> Result<BlueprintDraftView, String> {
    let draft = fetch_draft(db, draft_id).await?;
    let messages = fetch_messages(db, draft_id).await?;
    Ok(BlueprintDraftView { draft, messages })
}

async fn insert_message(
    db: &crate::db::Db,
    draft_id: &str,
    role: &str,
    content: &str,
    change_summary: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO blueprint_messages (id, draft_id, role, content, change_summary, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(draft_id)
    .bind(role)
    .bind(content)
    .bind(change_summary)
    .bind(now_str())
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 把草稿的 specs/tasklist 写回 DB（序列化进 JSON 列）+ 刷新 updated_at。
async fn persist_draft_body(
    db: &crate::db::Db,
    draft_id: &str,
    prd_markdown: &str,
    specs: &[BlueprintSpec],
    tasklist: &[BlueprintTask],
) -> Result<(), String> {
    let specs_json = serde_json::to_string(specs).map_err(|e| e.to_string())?;
    let tasklist_json = serde_json::to_string(tasklist).map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE blueprint_drafts
         SET prd_markdown = ?, specs_json = ?, tasklist_json = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(prd_markdown)
    .bind(&specs_json)
    .bind(&tasklist_json)
    .bind(now_str())
    .bind(draft_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 取一段文本的首行作为标题（截断 ~40 字），用于大需求列表展示。
fn derive_title(brief: &str) -> String {
    let line = brief.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("未命名需求");
    let trimmed: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        format!("{}…", trimmed)
    } else {
        trimmed
    }
}

/// 把用户勾选引用的项目文件读出来拼成一段上下文（守卫读取 + 注入过滤 + 单文件/总量封顶）。
fn build_ref_files_block(repo_path: &str, ref_files: &[String]) -> String {
    const TOTAL_CAP: usize = 24_000; // 引用总量上限，防把超大文件灌爆 prompt
    let mut buf = String::new();
    let mut used = 0usize;
    for rel in ref_files.iter().filter(|r| !r.trim().is_empty()).take(12) {
        if used >= TOTAL_CAP {
            break;
        }
        match crate::commands::project_context::read_repo_file(repo_path, rel.trim()) {
            Ok(content) if !crate::core::security::has_obvious_injection(&content) => {
                let budget = (TOTAL_CAP - used).min(8_000);
                let slice: String = content.chars().take(budget).collect();
                used += slice.len();
                buf.push_str(&format!("\n--- 文件：{} ---\n{}\n", rel.trim(), slice));
            }
            _ => { /* 读取失败/疑似注入：跳过该文件，不阻断起草 */ }
        }
    }
    buf
}

/// 步骤1：根据大需求起草一条**新的**蓝图草稿并持久（孵化台支持每项目多条大需求，不再清旧草稿）。
/// `ref_files`：用户勾选引用的项目文件（相对仓库根），读出后作为重点上下文注入。
#[tauri::command]
pub async fn start_blueprint_draft(
    project_id: String,
    brief: String,
    ref_files: Vec<String>,
    state: State<'_, AppState>,
) -> Result<BlueprintDraftView, String> {
    let brief = brief.trim().to_string();
    if brief.is_empty() {
        return Err("大需求内容不能为空".into());
    }
    if crate::core::security::has_obvious_injection(&brief) {
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

    // 用户勾选引用的项目文件（重点上下文）。
    let ref_block = if ref_files.is_empty() || project.repo_path.trim().is_empty() {
        String::new()
    } else {
        build_ref_files_block(&project.repo_path, &ref_files)
    };

    // 复用需求分析的项目上下文（claude.md/agents.md/技术文件/目录树），让蓝图贴合实际仓库。
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
    let ref_section = if ref_block.trim().is_empty() {
        String::new()
    } else {
        format!("\n用户特别引用的项目文件（请重点参考）：\n{}\n", ref_block)
    };

    let prompt = format!(
        r#"你是资深产品 + 架构师。下面是一个项目的「大需求」原始描述，请把它炼成一套初始项目蓝图。

项目名称：{name}
项目描述：{desc}

既有仓库上下文：
{ctx}
{refs}
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
        refs = ref_section,
        brief = brief,
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        BLUEPRINT_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的项目蓝图生成 Agent，把一段大需求拆解为 PRD、技术规格与可执行任务清单。只输出调用方要求的 JSON。"),
        Some(&project_id),
        Some(&brief),
    )
    .await
    .map_err(|e| format!("AI 生成失败: {}", e))?;

    let mut parsed = parse_raw(&raw)?;
    if parsed.prd_markdown.trim().is_empty()
        && parsed.specs.is_empty()
        && parsed.tasklist.is_empty()
    {
        return Err("AI 未能从该大需求生成有效蓝图，请补充更具体的描述后重试".into());
    }
    ensure_ids(&mut parsed.specs, &mut parsed.tasklist);

    let draft_id = Uuid::new_v4().to_string();
    let now = now_str();
    let title = derive_title(&brief);
    let specs_json = serde_json::to_string(&parsed.specs).map_err(|e| e.to_string())?;
    let tasklist_json = serde_json::to_string(&parsed.tasklist).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO blueprint_drafts
         (id, project_id, title, brief, prd_markdown, specs_json, tasklist_json, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'drafting', ?, ?)",
    )
    .bind(&draft_id)
    .bind(&project_id)
    .bind(&title)
    .bind(&brief)
    .bind(&parsed.prd_markdown)
    .bind(&specs_json)
    .bind(&tasklist_json)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    insert_message(&state.db, &draft_id, "user", &brief, "").await?;
    let summary = format!(
        "已根据大需求起草初始蓝图：PRD + {} 条规格 + {} 条任务。可在左侧继续对我说要改哪里。",
        parsed.specs.len(),
        parsed.tasklist.len()
    );
    insert_message(&state.db, &draft_id, "assistant", &summary, "起草初始蓝图").await?;

    load_view(&state.db, &draft_id).await
}

/// 步骤2：自然语言指令修正草稿。AI 收到当前草稿（含稳定 id）+ 对话历史 + 指令，
/// 回传整份更新草稿（保留 id，新增项可省略 id 由我们补），存库并记一轮对话。
#[tauri::command]
pub async fn refine_blueprint_draft(
    draft_id: String,
    instruction: String,
    state: State<'_, AppState>,
) -> Result<BlueprintDraftView, String> {
    let instruction = instruction.trim().to_string();
    if instruction.is_empty() {
        return Err("修正指令不能为空".into());
    }
    if crate::core::security::has_obvious_injection(&instruction) {
        return Err("指令文本疑似含注入内容，已拒绝".into());
    }

    let draft = fetch_draft(&state.db, &draft_id).await?;
    if draft.status == "coding" {
        return Err("该需求已进入编码开发，如需调整请在「变更审核」处理或新建一条需求改动".into());
    }

    // 当前草稿序列化为 JSON（带 id），供模型在其上做最小改动。
    let current = serde_json::json!({
        "prd_markdown": draft.prd_markdown,
        "specs": draft.specs,
        "tasklist": draft.tasklist,
    });
    let current_json =
        serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;

    // 取最近若干轮对话作为上下文（更早省略）。
    let history = fetch_messages(&state.db, &draft_id).await?;
    let recent: Vec<&BlueprintMessage> = history.iter().rev().take(HISTORY_CAP).collect();
    let mut hist_block = String::new();
    for m in recent.iter().rev() {
        let who = if m.role == "user" { "用户" } else { "你" };
        hist_block.push_str(&format!("{}：{}\n", who, m.content.trim()));
    }
    if hist_block.trim().is_empty() {
        hist_block.push_str("（无）\n");
    }

    let prompt = format!(
        r#"你正在与用户多轮打磨一份项目蓝图。下面是【当前蓝图】（JSON，规格与任务都带稳定 id）、【对话历史】与用户【本轮指令】。
请在当前蓝图基础上做**最小必要改动**满足指令，然后回传**整份更新后的蓝图**。

铁律：
- 未被指令涉及的内容**原样保留**，不要重写、不要删改。
- 修改已有规格/任务时**必须沿用其原 id**；新增项可不带 id（系统会补）；要删除某项就在数组里去掉它。
- category/severity 取值范围与原来一致（specs.category ∈ tech_stack/architecture/coding/api/testing；tasklist.category ∈ Feature/Bug/Refactor/Chore；severity ∈ low/medium/high）。
- change_summary 用一句话说明这轮改了什么（如「PRD 验收标准细化为 5 条；支付任务拆为 3 条」）。

【当前蓝图】
{current}

【对话历史】
{history}

【本轮指令】
{instruction}

严格只输出如下 JSON，不要任何额外文字或代码围栏：
{{
  "prd_markdown": "...",
  "specs": [{{"id":"...","category":"architecture","title":"...","content_markdown":"..."}}],
  "tasklist": [{{"id":"...","title":"...","description":"...","category":"Feature","severity":"medium"}}],
  "change_summary": "..."
}}"#,
        current = current_json,
        history = hist_block.trim(),
        instruction = instruction,
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        BLUEPRINT_SYSTEM_KIND,
        &prompt,
        Some("你是 AutoForge 的项目蓝图修正 Agent，在既有蓝图上做最小必要改动并回传整份更新结果。只输出调用方要求的 JSON。"),
        Some(&draft.project_id),
        Some(&instruction),
    )
    .await
    .map_err(|e| format!("AI 修正失败: {}", e))?;

    let mut parsed = parse_raw(&raw)?;
    if parsed.prd_markdown.trim().is_empty()
        && parsed.specs.is_empty()
        && parsed.tasklist.is_empty()
    {
        return Err("AI 未能产出有效的修正结果，请换种说法再试".into());
    }
    ensure_ids(&mut parsed.specs, &mut parsed.tasklist);

    persist_draft_body(
        &state.db,
        &draft_id,
        &parsed.prd_markdown,
        &parsed.specs,
        &parsed.tasklist,
    )
    .await?;

    let change_summary = if parsed.change_summary.trim().is_empty() {
        "已更新蓝图".to_string()
    } else {
        parsed.change_summary.trim().to_string()
    };
    insert_message(&state.db, &draft_id, "user", &instruction, "").await?;
    insert_message(&state.db, &draft_id, "assistant", &change_summary, &change_summary).await?;

    load_view(&state.db, &draft_id).await
}

/// 步骤2'：人工手改落库（前端发回编辑后的整份 PRD/规格/任务）。不记对话、不调 AI。
#[tauri::command]
pub async fn patch_blueprint_draft(
    draft_id: String,
    prd_markdown: String,
    specs: Vec<BlueprintSpec>,
    tasklist: Vec<BlueprintTask>,
    state: State<'_, AppState>,
) -> Result<BlueprintDraft, String> {
    let draft = fetch_draft(&state.db, &draft_id).await?;
    if draft.status == "coding" {
        return Err("该需求已进入编码开发，不可再手改".into());
    }
    // 人工输入同样过注入过滤（PRD 正文 + 各规格/任务文本）。
    if crate::core::security::has_obvious_injection(&prd_markdown) {
        return Err("PRD 文本疑似含注入内容，已拒绝".into());
    }
    for s in &specs {
        if crate::core::security::has_obvious_injection(&s.title)
            || crate::core::security::has_obvious_injection(&s.content_markdown)
        {
            return Err("规格文本疑似含注入内容，已拒绝".into());
        }
    }
    for t in &tasklist {
        if crate::core::security::has_obvious_injection(&t.title)
            || crate::core::security::has_obvious_injection(&t.description)
        {
            return Err("任务文本疑似含注入内容，已拒绝".into());
        }
    }

    let mut specs = specs;
    let mut tasklist = tasklist;
    ensure_ids(&mut specs, &mut tasklist);
    persist_draft_body(&state.db, &draft_id, &prd_markdown, &specs, &tasklist).await?;
    fetch_draft(&state.db, &draft_id).await
}

/// 按 draft_id 恢复一条大需求的草稿 + 对话历史（无则返回 null）。
#[tauri::command]
pub async fn get_blueprint_draft(
    draft_id: String,
    state: State<'_, AppState>,
) -> Result<Option<BlueprintDraftView>, String> {
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM blueprint_drafts WHERE id = ?")
            .bind(&draft_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    match exists {
        None => Ok(None),
        Some((id,)) => Ok(Some(load_view(&state.db, &id).await?)),
    }
}

/// 由 status + 关联 CR 现状派生列表展示态。
fn derive_display_status(status: &str, cr_status: Option<&str>) -> String {
    if status != "coding" {
        return status.to_string(); // drafting
    }
    match cr_status {
        Some(s) if s.contains("merged") => "implemented",
        Some(s) if s.contains("conflict") => "conflict",
        Some(s) if s.contains("fail") || s.contains("reject") => "failed",
        Some(s) if s.contains("review") => "in_review",
        _ => "coding", // executing / pending_execution / 未知
    }
    .to_string()
}

/// 孵化台左栏：列出某项目全部大需求草稿（含派生状态、规格/任务计数）。
#[tauri::command]
pub async fn list_blueprint_drafts(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<BlueprintDraftSummary>, String> {
    type Row = (String, String, String, String, String, String, String, Option<String>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT d.id, d.project_id, d.title, d.specs_json, d.tasklist_json, d.status, d.issue_id, c.status, d.updated_at
         FROM blueprint_drafts d
         LEFT JOIN change_requests c ON c.id = d.cr_id
         WHERE d.project_id = ?
         ORDER BY d.updated_at DESC",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, project_id, title, specs_json, tasklist_json, status, issue_id, cr_status, updated_at)| {
            let spec_count = serde_json::from_str::<Vec<BlueprintSpec>>(&specs_json)
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            let task_count = serde_json::from_str::<Vec<BlueprintTask>>(&tasklist_json)
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            let display_status = derive_display_status(&status, cr_status.as_deref());
            BlueprintDraftSummary {
                id,
                project_id,
                title,
                status,
                display_status,
                spec_count,
                task_count,
                issue_id,
                cr_id: String::new(),
                updated_at,
            }
        })
        .collect())
}

/// 删除一条大需求草稿（含其对话）。已进入编码的不再删除（保留回链）。
#[tauri::command]
pub async fn delete_blueprint_draft(
    draft_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let draft = fetch_draft(&state.db, &draft_id).await?;
    if draft.status == "coding" {
        return Err("该需求已进入编码开发，不可删除（请到「变更审核」处理）".into());
    }
    sqlx::query("DELETE FROM blueprint_messages WHERE draft_id = ?")
        .bind(&draft_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM blueprint_drafts WHERE id = ?")
        .bind(&draft_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 把蓝图内容（PRD + 规格 + 任务）拼成编码工单的「需求来源」段（work_context）。
fn compose_work_context(draft: &BlueprintDraft) -> String {
    let mut buf = String::new();
    buf.push_str("# 大需求原文\n\n");
    buf.push_str(draft.brief.trim());
    buf.push_str("\n\n# 产品需求文档（PRD）\n\n");
    buf.push_str(draft.prd_markdown.trim());
    if !draft.specs.is_empty() {
        buf.push_str("\n\n# 技术规格\n\n");
        for s in &draft.specs {
            buf.push_str(&format!(
                "## [{}] {}\n{}\n\n",
                s.category.trim(),
                s.title.trim(),
                s.content_markdown.trim()
            ));
        }
    }
    buf.push_str("\n# 任务清单\n\n");
    buf.push_str(&render_tasklist_md(&draft.tasklist));
    buf
}

/// 可选副作用：把 PRD 写入 .autoforge/docs、规格登记 specs（编码开发不依赖它，但方便留档）。
#[tauri::command]
pub async fn apply_blueprint_draft(
    draft_id: String,
    write_tasklist_doc: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let draft = fetch_draft(&state.db, &draft_id).await?;
    let repo_path: String =
        sqlx::query_as::<_, (String,)>("SELECT repo_path FROM projects WHERE id = ?")
            .bind(&draft.project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .map(|(p,)| p.trim().to_string())
            .ok_or("项目不存在")?;
    if repo_path.is_empty() {
        return Err("项目未设置本地仓库路径，无法写入工作区".into());
    }

    let mut written: Vec<String> = Vec::new();
    if !draft.prd_markdown.trim().is_empty() {
        crate::commands::workspace::write_workspace_path(&repo_path, "docs/PRD.md", &draft.prd_markdown).await?;
        written.push("docs/PRD.md".into());
    }
    if write_tasklist_doc && !draft.tasklist.is_empty() {
        let md = render_tasklist_md(&draft.tasklist);
        crate::commands::workspace::write_workspace_path(&repo_path, "docs/TASKLIST.md", &md).await?;
        written.push("docs/TASKLIST.md".into());
    }
    let spec_tuples: Vec<(String, String, String)> = draft
        .specs
        .iter()
        .filter(|s| !s.title.trim().is_empty() && !s.content_markdown.trim().is_empty())
        .map(|s| (s.category.clone(), s.title.clone(), s.content_markdown.clone()))
        .collect();
    let spec_count = if spec_tuples.is_empty() {
        0
    } else {
        crate::commands::specs::insert_db_specs(&draft.project_id, &spec_tuples, &state).await?
    };

    Ok(format!("已写入工作区文档 {} 个，登记规格 {} 条", written.len(), spec_count))
}

/// 编码开发：把当前大需求**直接**落为一条 issue + CR + 编码执行（跳过前置需求审核），
/// 蓝图内容作为 work_context 注入编码工单。代码审核（review_2）仍是合并唯一闸门。
/// 镜像会议室「立即编码」express 路径，不破坏「双审核 / 合并唯一入口」架构。
#[tauri::command]
pub async fn code_blueprint_draft(
    draft_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let draft = fetch_draft(&state.db, &draft_id).await?;
    if draft.status == "coding" {
        return Err("该需求已开始编码，请在「变更审核」查看进度".into());
    }
    if draft.prd_markdown.trim().is_empty() && draft.tasklist.is_empty() {
        return Err("蓝图内容为空，无法编码".into());
    }

    let title = if draft.title.trim().is_empty() {
        derive_title(&draft.brief)
    } else {
        draft.title.clone()
    };
    let description = draft.brief.trim().to_string();
    if crate::core::security::has_obvious_injection(&title)
        || crate::core::security::has_obvious_injection(&description)
    {
        return Err("需求内容包含可疑指令，已拦截".into());
    }

    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&draft.project_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("项目不存在")?;

    // 创建需求：express 路径直接落 pending_execution（不入分析/需求审核队列）。
    let issue_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO issues (id, project_id, source_type, title, description, category, status, source_ref)
         VALUES (?, ?, 'blueprint', ?, ?, 'Feature', 'pending_execution', ?)",
    )
    .bind(&issue_id)
    .bind(&draft.project_id)
    .bind(&title)
    .bind(&description)
    .bind(&draft_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let issue =
        sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&issue_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let work_context = compose_work_context(&draft);
    let cr = crate::commands::change_requests::create_cr_for_issue(
        &state.db,
        &state.job_tx,
        &issue,
        &project,
        Some("孵化台「编码开发」：操作者确认蓝图后直接进入编码，代码审核(review_2)仍为合并闸门"),
        "admin",
        Some(&work_context),
    )
    .await?;

    sqlx::query("UPDATE blueprint_drafts SET status='coding', issue_id=?, cr_id=?, updated_at=? WHERE id=?")
        .bind(&issue_id)
        .bind(&cr.id)
        .bind(now_str())
        .bind(&draft_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    crate::core::event::emit(
        &app,
        crate::core::event::AppEvent::IssueCreated {
            issue_id: issue_id.clone(),
            project_id: draft.project_id.clone(),
        },
    );

    Ok(cr.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_strips_surrounding_prose_and_fences() {
        // 模型常在 JSON 前后加解释或代码围栏：parse_raw 应只取首 { 到末 }。
        let raw = "好的，这是蓝图：\n```json\n{\"prd_markdown\":\"# A\",\"specs\":[],\"tasklist\":[]}\n```\n以上。";
        let p = parse_raw(raw).expect("should parse");
        assert_eq!(p.prd_markdown, "# A");
        assert!(p.specs.is_empty() && p.tasklist.is_empty());
    }

    #[test]
    fn parse_raw_errors_when_no_json() {
        assert!(parse_raw("没有任何大括号").is_err());
    }

    #[test]
    fn spec_and_task_deserialize_with_missing_id_and_defaults() {
        // 起草轮模型不带 id，且 task 可缺 category/severity —— 必须回落默认且解析不报错。
        let json = r#"{
            "prd_markdown": "p",
            "specs": [{"category":"api","title":"t","content_markdown":"c"}],
            "tasklist": [{"title":"做一件事"}]
        }"#;
        let p = parse_raw(json).expect("parse");
        assert_eq!(p.specs[0].id, "");
        assert_eq!(p.tasklist[0].category, "Feature");
        assert_eq!(p.tasklist[0].severity, "medium");
    }

    #[test]
    fn ensure_ids_fills_blanks_but_preserves_existing() {
        let mut specs = vec![
            BlueprintSpec { id: "keep-1".into(), category: "api".into(), title: "a".into(), content_markdown: "x".into() },
            BlueprintSpec { id: "".into(), category: "coding".into(), title: "b".into(), content_markdown: "y".into() },
        ];
        let mut tasks = vec![BlueprintTask {
            id: "  ".into(), title: "t".into(), description: "".into(),
            category: "Bug".into(), severity: "high".into(),
        }];
        ensure_ids(&mut specs, &mut tasks);
        assert_eq!(specs[0].id, "keep-1", "已有 id 必须原样保留");
        assert!(!specs[1].id.trim().is_empty(), "空 id 必须补齐");
        assert!(!tasks[0].id.trim().is_empty(), "纯空白 id 视为空、补齐");
        assert_ne!(specs[1].id, tasks[0].id, "补齐的 id 应各不相同");
    }

    #[test]
    fn render_tasklist_md_shapes_checklist() {
        let tasks = vec![BlueprintTask {
            id: "1".into(), title: "实现登录".into(), description: "支持邮箱登录".into(),
            category: "Feature".into(), severity: "medium".into(),
        }];
        let md = render_tasklist_md(&tasks);
        assert!(md.contains("- [ ] **实现登录** `Feature`/`medium`"));
        assert!(md.contains("  - 支持邮箱登录"));
    }

    #[test]
    fn draft_round_trips_through_json_columns() {
        // row_to_draft 把 *_json 文本还原为结构化草稿，且坏 JSON 不 panic（回落空）。
        let specs = vec![BlueprintSpec { id: "s1".into(), category: "api".into(), title: "t".into(), content_markdown: "c".into() }];
        let specs_json = serde_json::to_string(&specs).unwrap();
        let d = row_to_draft(
            "d1".into(), "p1".into(), "需求标题".into(), "brief".into(), "# prd".into(),
            specs_json, "not json".into(), "drafting".into(), "".into(), "".into(), "t0".into(), "t1".into(),
        );
        assert_eq!(d.specs.len(), 1);
        assert_eq!(d.specs[0].id, "s1");
        assert!(d.tasklist.is_empty(), "坏 JSON 应回落空而非 panic");
    }
}
