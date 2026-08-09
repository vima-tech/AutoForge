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

/// P3 蓝图评审结果（孵化台深化 §3.3 默认多轮评估）：四维打分 + 待补强项 + 总评。
/// critic（`spec_grader` 角色）落稿后打分；低于阈值且有轮次预算则回喂起草 Agent 自动修订。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct BlueprintEval {
    #[serde(default)]
    pub prd_completeness: i64,
    #[serde(default)]
    pub spec_executability: i64,
    #[serde(default)]
    pub task_granularity: i64,
    #[serde(default)]
    pub code_fit: i64,
    #[serde(default)]
    pub gaps: Vec<String>,
    #[serde(default)]
    pub summary: String,
}

impl BlueprintEval {
    /// 四维最低分（短板决定是否需再修订）。
    pub fn min_score(&self) -> i64 {
        self.prd_completeness
            .min(self.spec_executability)
            .min(self.task_granularity)
            .min(self.code_fit)
    }
    /// 是否达标（所有维度 ≥ 阈值）。
    pub fn passes(&self, threshold: i64) -> bool {
        self.min_score() >= threshold
    }
}

/// 从 critic 原始输出解析评估 JSON（容忍前后解释/围栏，取首 `{` 到末 `}`）。
pub(crate) fn parse_eval(raw: &str) -> Option<BlueprintEval> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(raw.get(start..=end)?).ok()
}

/// 落库一次评估：写 `eval_json` + 记一条 `role='eval'` 消息（总评）。可测。
pub(crate) async fn store_eval(
    db: &crate::db::Db,
    draft_id: &str,
    eval: &BlueprintEval,
) -> Result<(), String> {
    let json = serde_json::to_string(eval).map_err(|e| e.to_string())?;
    sqlx::query("UPDATE blueprint_drafts SET eval_json=?, updated_at=? WHERE id=?")
        .bind(&json)
        .bind(now_str())
        .bind(draft_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    let note = if eval.summary.trim().is_empty() {
        format!("评估：最低分 {}/10", eval.min_score())
    } else {
        eval.summary.trim().to_string()
    };
    insert_message(db, draft_id, "eval", &note, "").await?;
    Ok(())
}

/// P3 评估开关（app_settings 键 `blueprint.eval_enabled`，默认关=旧行为零回归）。
async fn blueprint_eval_enabled(db: &crate::db::Db) -> bool {
    sqlx::query_as::<_, (String,)>("SELECT value FROM app_settings WHERE key='blueprint.eval_enabled'")
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|(v,)| v == "true" || v == "1")
        .unwrap_or(false)
}

/// 落稿后自动评审（P3 默认多轮评估的评审步）：spec_grader 四维打分 → 落 eval_json + eval 消息。
/// best-effort：LLM 不可用 / 解析失败则跳过，不阻断起草主流程。LLM 行为需运行时验证；
/// 本函数的解析/落库逻辑由 `parse_eval`/`store_eval` 单测覆盖。
pub(crate) async fn run_blueprint_critic(
    db: &crate::db::Db,
    project_id: &str,
    draft_id: &str,
) -> Result<(), String> {
    let draft = fetch_draft(db, draft_id).await?;
    let specs_json = serde_json::to_string(&draft.specs).unwrap_or_default();
    let tasks_json = serde_json::to_string(&draft.tasklist).unwrap_or_default();
    let prompt = format!(
        "请评审下面这份大需求蓝图并按四维打分（只输出评估 JSON）。\n\n【PRD】\n{}\n\n【规格】\n{}\n\n【任务清单】\n{}",
        draft.prd_markdown, specs_json, tasks_json
    );
    let raw = crate::agents::llm::run_system_role_text(
        db,
        "spec_grader",
        &prompt,
        crate::agents::roles::builtin_prompt("spec_grader"),
        Some(project_id),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    if let Some(eval) = parse_eval(&raw) {
        store_eval(db, draft_id, &eval).await?;
    }
    Ok(())
}

/// P1 grounding（孵化台深化 §3.1/§3.4）：起草/修正前，从统一上下文基质 assemble 一小段
/// 与本项目相关的已有上下文（需求 / 编码执行日志 / 审核意见 / 既有草稿），注入 prompt 作为
/// grounding，让起草 Agent 看到「项目此前发生过什么」而非凭空生成。
///
/// 复用编码台取景框（issue/spec/code_agent_log/llm_trace）+ 小预算（~6KB）；正文经保尾摘要。
/// 防御性再过一遍注入闸（源头 intake 已过滤，此处兜底）；无基质条目时返回空串（prompt 不变，
/// 即旧行为，零回归）。基质空/查询失败均静默降级。
async fn build_substrate_grounding(db: &crate::db::Db, project_id: &str) -> String {
    use crate::core::{context, lens};
    let preset = lens::preset_for_role("coding");
    let req = context::ContextRequest {
        project_id: project_id.to_string(),
        include: preset.include,
        refs: vec![],
        budget_bytes: 6000,
    };
    let items = match context::assemble(db, &req).await {
        Ok(v) if !v.is_empty() => v,
        _ => return String::new(),
    };
    let mut out = String::new();
    for it in items.iter().take(6) {
        let snip = context::fetch_content(db, it, 300).await.unwrap_or_default();
        let snip = snip.trim();
        // 源头已过注入闸，此处兜底：疑似注入的条目跳过，不喂进起草 prompt。
        if snip.is_empty() || crate::core::security::has_obvious_injection(snip) {
            continue;
        }
        out.push_str(&format!("- [{}] {}：{}\n", it.source_kind, it.title, snip));
    }
    if out.is_empty() {
        return String::new();
    }
    format!(
        "\n【项目已有上下文（来自统一基质，供参考理解现状，勿照抄）】\n{out}"
    )
}

/// 追问挂起（孵化台深化 §3.2 断点续跑）：起草 Agent 调 `ask_user` 终止型工具时，把问题
/// 落 `blueprint_messages(role='question')`，草稿置 `awaiting_answer` + `pending_question`。
/// 纯状态转换，供 P2 工具循环收口调用；本身可独立单测。
pub(crate) async fn set_awaiting_answer(
    db: &crate::db::Db,
    draft_id: &str,
    question: &str,
) -> Result<(), String> {
    insert_message(db, draft_id, "question", question, "").await?;
    sqlx::query(
        "UPDATE blueprint_drafts SET status='awaiting_answer', pending_question=?, updated_at=? WHERE id=?",
    )
    .bind(question)
    .bind(now_str())
    .bind(draft_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 回答追问的状态转换（可测 helper）：记录答复 → 清 `pending_question` → 状态回 `drafting`。
/// **续跑机制**（§3.2）：不在此保存运行时状态；下一轮 `refine_blueprint_draft` 从
/// `blueprint_messages` 重建 transcript（此刻已含 Q&A）再起工具循环，天然幂等。
pub(crate) async fn apply_answer(
    db: &crate::db::Db,
    draft_id: &str,
    answer: &str,
) -> Result<(), String> {
    insert_message(db, draft_id, "answer", answer, "").await?;
    sqlx::query(
        "UPDATE blueprint_drafts SET status='drafting', pending_question='', updated_at=? WHERE id=?",
    )
    .bind(now_str())
    .bind(draft_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 命令：回答孵化台起草 Agent 的追问，清挂起态、返回更新后的草稿视图（断点续跑，见 §3.2）。
#[tauri::command]
pub async fn answer_blueprint_question(
    draft_id: String,
    answer: String,
    state: State<'_, AppState>,
) -> Result<BlueprintDraftView, String> {
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        return Err("答复不能为空".into());
    }
    if crate::core::security::has_obvious_injection(&answer) {
        return Err("答复文本疑似含注入内容，已拒绝".into());
    }
    apply_answer(&state.db, &draft_id, &answer).await?;
    load_view(&state.db, &draft_id).await
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

    // 上下文基质登记（基质 §3.2：孵化台草稿此前与会议室/CR 上下文不互通，是关键缺口）。
    // 落稿即把该大需求草稿投影为 ContextItem，让编码/审核/会议室等环节可取用其 PRD。
    // best-effort：查 draft 归属项目/标题后登记；content_ref=bp:<id> 对应 fetch_content 的 bp 读取器。
    if let Some((project_id, title)) =
        sqlx::query_as::<_, (String, String)>("SELECT project_id, title FROM blueprint_drafts WHERE id=?")
            .bind(draft_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
    {
        let cref = format!("bp:{draft_id}");
        let disp = if title.trim().is_empty() { "孵化台草稿" } else { title.trim() };
        let _ = crate::core::context::register(
            db,
            crate::core::context::NewContextItem {
                project_id: &project_id,
                source_kind: crate::core::context::source_kind::INCUBATOR_DRAFT,
                source_id: draft_id,
                title: disp,
                origin_stage: "requirement",
                origin_actor: "spec_writer",
                content_ref: &cref,
                size_hint: prd_markdown.len() as i64,
                trust: crate::core::context::trust::TRUSTED,
                labels: "[]",
            },
        )
        .await;
    }
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

/// 跑「分析后端」产出蓝图 JSON 原文。`backend = Some("code_agent")` 且项目有仓库路径时，用项目
/// 已配置的编码 Agent（execution 同款 `resolve`）**只读跑真实仓库**起草——产出的 PRD/规格/任务
/// 更贴实际代码；否则（默认 / `"analysis"`）走需求分析专家 LLM（`BLUEPRINT_SYSTEM_KIND`）。
/// 编码 Agent 不可用 / 空输出 / 失败时**回落 LLM**，绝不让起草整体失败。
async fn run_blueprint_backend(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
    backend: Option<&str>,
    prompt: &str,
    system: &str,
    recall_key: &str,
) -> Result<String, String> {
    let want_code_agent =
        matches!(backend, Some("code_agent")) && !project.repo_path.trim().is_empty();
    if want_code_agent {
        let agent = crate::agents::code_agent::resolve(db, project).await;
        let (wall_secs, idle_secs) = crate::commands::system::load_execution_limits(db).await;
        let limits = crate::agents::code_agent::RunLimits { wall_secs, idle_secs };
        let mcp = crate::agents::code_agent::load_code_agent_mcp(db).await;
        match agent.answer(&project.repo_path, prompt, limits, &mcp, None).await {
            Ok(t) if !t.trim().is_empty() => return Ok(t),
            Ok(_) => tracing::warn!("[blueprint] 编码 Agent 输出为空，回落需求分析专家"),
            Err(e) => {
                tracing::warn!("[blueprint] 编码 Agent 起草失败，回落需求分析专家: {}", e)
            }
        }
    }
    crate::agents::llm::run_system_role_text(
        db,
        BLUEPRINT_SYSTEM_KIND,
        prompt,
        Some(system),
        Some(&project.id),
        Some(recall_key),
    )
    .await
    .map_err(|e| format!("AI 生成失败: {}", e))
}

/// 步骤1：根据大需求起草一条**新的**蓝图草稿并持久（孵化台支持每项目多条大需求，不再清旧草稿）。
/// `ref_files`：用户勾选引用的项目文件（相对仓库根），读出后作为重点上下文注入。
/// `backend`：分析后端——`"analysis"`（默认，需求分析专家 LLM）或 `"code_agent"`（编码 Agent 读仓库）。
#[tauri::command]
pub async fn start_blueprint_draft(
    project_id: String,
    brief: String,
    ref_files: Vec<String>,
    backend: Option<String>,
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

    let raw = run_blueprint_backend(
        &state.db,
        &project,
        backend.as_deref(),
        &prompt,
        "你是 AutoForge 的项目蓝图生成 Agent，把一段大需求拆解为 PRD、技术规格与可执行任务清单。只输出调用方要求的 JSON。",
        &brief,
    )
    .await?;

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
    backend: Option<String>,
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
    // 加载项目（编码 Agent 后端起草需要仓库路径；LLM 后端只用其 id 走召回）。
    let project = sqlx::query_as::<_, crate::models::project::Project>(
        "SELECT * FROM projects WHERE id=?",
    )
    .bind(&draft.project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or("项目不存在")?;

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

    // P1 grounding：注入项目已有基质上下文（无则空串，prompt 不变=旧行为）。
    let grounding = build_substrate_grounding(&state.db, &draft.project_id).await;

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
{grounding}
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
        grounding = grounding,
        instruction = instruction,
    );

    let raw = run_blueprint_backend(
        &state.db,
        &project,
        backend.as_deref(),
        &prompt,
        "你是 AutoForge 的项目蓝图修正 Agent，在既有蓝图上做最小必要改动并回传整份更新结果。只输出调用方要求的 JSON。",
        &instruction,
    )
    .await?;

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

    // P3：若开启评估，落稿后自动跑 critic 打分（best-effort，不阻断；开关默认关=零回归）。
    if blueprint_eval_enabled(&state.db).await {
        let _ = run_blueprint_critic(&state.db, &draft.project_id, &draft_id).await;
    }

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

    /// P3 评估解析 + 阈值：容忍围栏；min_score 取四维最低；passes 全维达标才 true。
    #[test]
    fn eval_parse_and_threshold() {
        let raw = "评估如下：\n```json\n{\"prd_completeness\":8,\"spec_executability\":5,\"task_granularity\":9,\"code_fit\":7,\"gaps\":[\"验收标准缺量化\"],\"summary\":\"整体可用，规格偏空\"}\n```";
        let e = parse_eval(raw).expect("parse eval");
        assert_eq!(e.min_score(), 5, "四维最低=规格可执行性 5");
        assert!(!e.passes(7), "阈值 7 → 短板 5 不达标");
        assert!(e.passes(5), "阈值 5 → 达标");
        assert_eq!(e.gaps.len(), 1);
    }

    /// P3 落库：store_eval 写 eval_json + 记 role='eval' 消息。
    #[tokio::test]
    async fn store_eval_persists_json_and_message() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE blueprint_drafts (id TEXT PRIMARY KEY, eval_json TEXT NOT NULL DEFAULT '', updated_at TEXT)")
            .execute(&db).await.unwrap();
        sqlx::query("CREATE TABLE blueprint_messages (id TEXT PRIMARY KEY, draft_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL DEFAULT '', change_summary TEXT NOT NULL DEFAULT '', created_at TEXT)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO blueprint_drafts (id) VALUES ('d1')").execute(&db).await.unwrap();

        let eval = BlueprintEval { prd_completeness: 8, spec_executability: 6, task_granularity: 7, code_fit: 7, gaps: vec![], summary: "还行".into() };
        store_eval(&db, "d1", &eval).await.unwrap();
        let (json,): (String,) = sqlx::query_as("SELECT eval_json FROM blueprint_drafts WHERE id='d1'").fetch_one(&db).await.unwrap();
        assert!(json.contains("\"spec_executability\":6"));
        let (role,): (String,) = sqlx::query_as("SELECT role FROM blueprint_messages WHERE draft_id='d1'").fetch_one(&db).await.unwrap();
        assert_eq!(role, "eval");
    }

    /// P1 grounding：从基质 assemble 出的项目上下文被注入起草 prompt；空项目返回空串（旧行为）。
    #[tokio::test]
    async fn substrate_grounding_injects_project_context() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE context_index (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                origin_stage TEXT NOT NULL DEFAULT '', origin_actor TEXT NOT NULL DEFAULT '',
                content_ref TEXT NOT NULL DEFAULT '', size_hint INTEGER NOT NULL DEFAULT 0,
                trust TEXT NOT NULL DEFAULT 'trusted', labels TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&db)
        .await
        .unwrap();
        crate::core::context::register(
            &db,
            crate::core::context::NewContextItem::trusted(
                "p1",
                crate::core::context::source_kind::CODE_AGENT_LOG,
                "l1",
                "上次编码修了登录 bug",
                "",
            ),
        )
        .await
        .unwrap();

        let g = build_substrate_grounding(&db, "p1").await;
        assert!(g.contains("上次编码修了登录 bug"), "基质上下文注入 grounding");
        assert!(g.contains("项目已有上下文"));

        let empty = build_substrate_grounding(&db, "none").await;
        assert!(empty.is_empty(), "空项目 → 空 grounding（prompt 不变=旧行为）");
    }

    /// 追问状态机（P2 断点续跑）：ask_user 挂起 → awaiting_answer + pending_question；
    /// 回答 → 清挂起、回 drafting；Q&A 均进 transcript 供下轮 refine 重建续跑。
    #[tokio::test]
    async fn awaiting_answer_state_machine_roundtrip() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE blueprint_drafts (id TEXT PRIMARY KEY, project_id TEXT,
             status TEXT NOT NULL DEFAULT 'drafting', pending_question TEXT NOT NULL DEFAULT '',
             updated_at TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE blueprint_messages (id TEXT PRIMARY KEY, draft_id TEXT NOT NULL,
             role TEXT NOT NULL, content TEXT NOT NULL DEFAULT '',
             change_summary TEXT NOT NULL DEFAULT '', created_at TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO blueprint_drafts (id, project_id, status) VALUES ('d1','p1','drafting')")
            .execute(&db)
            .await
            .unwrap();

        set_awaiting_answer(&db, "d1", "需要支持第三方登录吗？").await.unwrap();
        let (status, pending): (String, String) =
            sqlx::query_as("SELECT status, pending_question FROM blueprint_drafts WHERE id='d1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(status, "awaiting_answer");
        assert_eq!(pending, "需要支持第三方登录吗？");

        apply_answer(&db, "d1", "是，支持微信登录").await.unwrap();
        let (status2, pending2): (String, String) =
            sqlx::query_as("SELECT status, pending_question FROM blueprint_drafts WHERE id='d1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(status2, "drafting", "回答后回到起草态");
        assert_eq!(pending2, "", "pending_question 已清");

        let roles: Vec<String> =
            sqlx::query_as::<_, (String,)>("SELECT role FROM blueprint_messages WHERE draft_id='d1'")
                .fetch_all(&db)
                .await
                .unwrap()
                .into_iter()
                .map(|(r,)| r)
                .collect();
        assert!(roles.contains(&"question".to_string()));
        assert!(roles.contains(&"answer".to_string()));
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
