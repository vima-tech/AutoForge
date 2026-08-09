use crate::models::project::Project;
use crate::models::prototype::PrototypePrompt;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

/// 一个可作为原型设计依据的核心文档源（孵化台深化 §3.5B）。
/// 前端「关联文档」面板据此让用户勾选，`generate_prototype_prompt(doc_refs)` 按选中项拼上下文。
#[derive(Debug, Clone, Serialize)]
pub struct DocSource {
    /// design_md / blueprint_prd / spec / workspace
    pub kind: String,
    /// draft_id / category / rel_path（design_md 为空）
    pub r#ref: String,
    pub title: String,
    pub summary: String,
    pub est_tokens: i64,
    pub default_on: bool,
}

fn est_tokens(text: &str) -> i64 {
    // 粗估：中英文混排约 3-4 字符/token，取 /3 保守偏高。
    (text.chars().count() as i64 / 3).max(1)
}

/// 汇总一个项目当前所有可作为原型设计依据的核心文档源（P4：设计契约 / 需求 PRD / 技术规格）。
/// 供前端「关联文档」面板勾选；`generate_prototype_prompt` 再按选中的 `doc_refs` 拼上下文。
#[tauri::command]
pub async fn list_prototype_doc_sources(
    project_id: String,
    draft_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<DocSource>, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("项目不存在")?;
    Ok(collect_doc_sources(&state.db, &project_id, &project.repo_path, draft_id.as_deref()).await)
}

/// 汇总核心文档源的纯逻辑（DB + 文件驱动，命令外可测）。
pub(crate) async fn collect_doc_sources(
    db: &crate::db::Db,
    project_id: &str,
    repo_path: &str,
    draft_id: Option<&str>,
) -> Vec<DocSource> {
    let mut out: Vec<DocSource> = Vec::new();

    // ① 设计契约：DESIGN.md（必选）。
    if let Some(design) = read_repo_design(repo_path) {
        out.push(DocSource {
            kind: "design_md".into(),
            r#ref: String::new(),
            title: "DESIGN.md（设计契约）".into(),
            summary: "项目 UI 设计系统与 token 契约".into(),
            est_tokens: est_tokens(&design),
            default_on: true,
        });
    }

    // ② 需求文档：孵化台草稿 PRD（从孵化台跳入时默认选中）。
    if let Some(did) = draft_id.filter(|s| !s.is_empty()) {
        if let Some((title, prd)) =
            sqlx::query_as::<_, (String, String)>("SELECT title, prd_markdown FROM blueprint_drafts WHERE id=?")
                .bind(did)
                .fetch_optional(db)
                .await
                .ok()
                .flatten()
        {
            let disp = if title.trim().is_empty() { "孵化台草稿" } else { title.trim() };
            out.push(DocSource {
                kind: "blueprint_prd".into(),
                r#ref: did.to_string(),
                title: format!("需求 PRD · {disp}"),
                summary: "孵化台梳理的大需求 PRD".into(),
                est_tokens: est_tokens(&prd),
                default_on: true,
            });
        }
    }

    // ③ 技术规格：project_specs 按分类聚合（architecture/api 默认选中）。
    let specs: Vec<(String, String)> = sqlx::query_as(
        "SELECT category, title FROM project_specs WHERE project_id=? ORDER BY category",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (cat, _title) in &specs {
        if !seen.insert(cat.clone()) {
            continue; // 同分类只出一条聚合项
        }
        out.push(DocSource {
            kind: "spec".into(),
            r#ref: cat.clone(),
            title: format!("技术规格 · {cat}"),
            summary: "项目规格约束".into(),
            est_tokens: 0,
            default_on: matches!(cat.as_str(), "architecture" | "api"),
        });
    }

    out
}

/// 按选中的文档源引用读取正文，拼成 design_ctx（P4 §3.5C）。
/// ref 形如 `design_md` / `blueprint_prd:<id>` / `spec:<category>`；单条 ~5K、总量 ~18K 封顶；
/// 防御性过注入闸（疑似注入的文档跳过，不喂进原型 prompt）。
pub(crate) async fn read_doc_refs(
    db: &crate::db::Db,
    repo_path: &str,
    project_id: &str,
    doc_refs: &[String],
) -> String {
    const PER: usize = 5000;
    const TOTAL: usize = 18000;
    let mut ctx = String::new();
    let mut used = 0usize;
    for r in doc_refs {
        if used >= TOTAL {
            break;
        }
        let (header, body): (String, String) = if r == "design_md" {
            ("# 设计契约(DESIGN.md)".into(), read_repo_design(repo_path).unwrap_or_default())
        } else if let Some(id) = r.strip_prefix("blueprint_prd:") {
            let prd = sqlx::query_as::<_, (String,)>("SELECT prd_markdown FROM blueprint_drafts WHERE id=?")
                .bind(id)
                .fetch_optional(db)
                .await
                .ok()
                .flatten()
                .map(|(v,)| v)
                .unwrap_or_default();
            ("# 需求文档(PRD)".into(), prd)
        } else if let Some(cat) = r.strip_prefix("spec:") {
            let content = sqlx::query_as::<_, (String,)>(
                "SELECT group_concat(content, char(10)) FROM project_specs WHERE project_id=? AND category=?",
            )
            .bind(project_id)
            .bind(cat)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .and_then(|(v,)| Some(v))
            .unwrap_or_default();
            (format!("# 技术规格·{cat}"), content)
        } else {
            continue; // 未知 ref（如 workspace:）暂跳过
        };
        let body = body.trim();
        if body.is_empty() || crate::core::security::has_obvious_injection(body) {
            continue;
        }
        let budget = (TOTAL - used).min(PER);
        let slice: String = body.chars().take(budget).collect();
        used += slice.len();
        if !ctx.is_empty() {
            ctx.push_str("\n\n");
        }
        ctx.push_str(&header);
        ctx.push('\n');
        ctx.push_str(&slice);
    }
    ctx
}

/// 读取「改现有页面」选中的仓库页面组件源码，拼成「现有页面基础」块。
/// 守卫读（read_repo_file，防越界）+ 注入过滤 + 单条/总量封顶。
async fn read_existing_pages(repo_path: &str, refs: &[String]) -> String {
    const PER: usize = 6000;
    const TOTAL: usize = 20000;
    let mut buf = String::new();
    let mut used = 0usize;
    for rel in refs.iter().filter(|r| !r.trim().is_empty()).take(6) {
        if used >= TOTAL {
            break;
        }
        match crate::commands::project_context::read_repo_file(repo_path, rel.trim()) {
            Ok(content) if !crate::core::security::has_obvious_injection(&content) => {
                let budget = (TOTAL - used).min(PER);
                let slice: String = content.chars().take(budget).collect();
                used += slice.len();
                buf.push_str(&format!("\n### 文件：{}\n```\n{}\n```\n", rel.trim(), slice));
            }
            _ => { /* 读失败/疑似注入：跳过该文件，不阻断生成 */ }
        }
    }
    buf
}

#[tauri::command]
pub async fn list_prototype_prompts(
    project_id: Option<String>,
    // 给了 draft_id 则只列该大需求的原型（按需求归档；从孵化台跳入时传）。
    draft_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PrototypePrompt>, String> {
    if let Some(did) = draft_id.as_deref().filter(|s| !s.is_empty()) {
        return sqlx::query_as::<_, PrototypePrompt>(
            "SELECT * FROM prototype_prompts WHERE draft_id=? ORDER BY created_at DESC LIMIT 200",
        )
        .bind(did)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string());
    }
    match project_id {
        Some(pid) => sqlx::query_as::<_, PrototypePrompt>(
            "SELECT * FROM prototype_prompts WHERE project_id=? ORDER BY created_at DESC LIMIT 200",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        None => sqlx::query_as::<_, PrototypePrompt>(
            "SELECT * FROM prototype_prompts ORDER BY created_at DESC LIMIT 200",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn delete_prototype_prompt(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM prototype_prompts WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Refine (manually edit) a generated design prompt.
#[tauri::command]
pub async fn update_prototype_prompt(
    id: String,
    title: String,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<PrototypePrompt, String> {
    sqlx::query("UPDATE prototype_prompts SET title=?, prompt=? WHERE id=?")
        .bind(&title)
        .bind(&prompt)
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let row = sqlx::query_as::<_, PrototypePrompt>("SELECT * FROM prototype_prompts WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // Innate: 用户手动「完善」的原型提示词 = 该项目偏好的设计风格样本，捕获给原型角色召回。
    let content = format!("用户认可/完善后的原型设计提示词「{}」：\n\n{}", row.title, row.prompt);
    let trigger = "为该项目生成原型设计提示词时偏好的风格与结构".to_string();
    crate::knowledge::kb_add(&row.project_id, &content, &trigger).await;

    Ok(row)
}

/// Node 03 — generate a design prompt usable directly in OpenDesign / Stitch / Claude Design.
#[tauri::command]
pub async fn generate_prototype_prompt(
    project_id: String,
    issue_id: Option<String>,
    tool_target: Option<String>,
    draft_id: Option<String>,
    doc_refs: Option<Vec<String>>,
    // 'new'（新页面，默认）/ 'existing'（在现有页面基础上改动）。
    design_mode: Option<String>,
    // design_mode='existing' 时选中的现有页面组件仓库相对路径。
    existing_page_refs: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<PrototypePrompt, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project_id))?;

    // 硬约束：原型提示词必须对应一个孵化台需求（draft）。空 draft_id 直接拒绝，
    // 且不落库——杜绝生成脱离孵化台需求的「野」提示词（前端也会禁用生成按钮兜住）。
    let draft_id = draft_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or("原型提示词必须对应一个需求，请先在「需求孵化台」选择或新建一条需求")?;
    let (draft_title, draft_brief) = sqlx::query_as::<_, (String, String)>(
        "SELECT title, brief FROM blueprint_drafts WHERE id=?",
    )
    .bind(&draft_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or("对应的需求不存在（可能已删除），请重新选择")?;

    // Pull the project's spec docs as design context (best-effort).
    let specs: String = sqlx::query_as::<_, (String, String)>(
        "SELECT title, content FROM project_specs WHERE project_id=? ORDER BY updated_at DESC LIMIT 5",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(t, c)| format!("## {t}\n{}", c.chars().take(800).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n\n")
    })
    .unwrap_or_default();

    // The project's own DESIGN.md is the gold-standard design contract — feed it
    // verbatim so the generated prompt matches its depth and stays on-brand.
    let design_md = read_repo_design(&project.repo_path);

    // Combine the design system references the model must honour.
    let mut design_ctx = String::new();
    if let Some(d) = &design_md {
        design_ctx.push_str("# DESIGN.md（项目设计契约 · 必须严格对标其详细程度与风格）\n");
        design_ctx.push_str(&d.chars().take(8000).collect::<String>());
    }
    if !specs.trim().is_empty() {
        if !design_ctx.is_empty() {
            design_ctx.push_str("\n\n");
        }
        design_ctx.push_str("# 项目技术规格\n");
        design_ctx.push_str(&specs);
    }

    // P4 §3.5C：若前端「关联文档」面板给了显式 doc_refs，按选中文档拼 design_ctx（更准），
    // 覆盖上面的默认 DESIGN.md+specs 粗放种子；空则回落旧行为（向后兼容）。
    let doc_refs = doc_refs.unwrap_or_default();
    if !doc_refs.is_empty() {
        let picked = read_doc_refs(&state.db, &project.repo_path, &project_id, &doc_refs).await;
        if !picked.trim().is_empty() {
            design_ctx = picked;
        }
    }

    // 「改现有页面」模式：读选中的现有页面组件源码，作为改动基础前置进 design_ctx，
    // 并指令模型在其上做增量改动而非从零重设计（新页面模式则不注入，行为不变）。
    let mode = design_mode.unwrap_or_default();
    if mode == "existing" {
        let refs = existing_page_refs.clone().unwrap_or_default();
        let pages = read_existing_pages(&project.repo_path, &refs).await;
        if !pages.trim().is_empty() {
            design_ctx = format!(
                "# ⚠️ 本次是对【现有页面】的改动，不是新建页面\n\
                 必须在下面现有页面的基础上做**增量改动**：保持其整体布局、组件层级、交互流与\
                 视觉风格一致，只针对下方需求描述涉及的部分做修改/新增，不要从零重新设计整个页面。\n\n\
                 ## 现有页面代码（改动基础，务必延续其结构与风格）\n{pages}\n\n---\n\n{design_ctx}"
            );
        }
    }

    let target = tool_target.unwrap_or_else(|| "generic".to_string());
    // 从（必选的）孵化台草稿派生 feature 标题/描述（真实需求，而非「项目名+产品界面」）。
    let feature_title = if draft_title.trim().is_empty() {
        format!("{} 界面", project.name)
    } else {
        draft_title
    };
    let feature_desc = draft_brief;

    let heuristic = heuristic_prompt(&project.name, &target, &feature_title, &feature_desc, &design_ctx);
    let prompt = llm_prompt(&state.db, &project.id, &project.name, &target, &feature_title, &feature_desc, &design_ctx)
        .await
        .unwrap_or(heuristic);

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO prototype_prompts (id, project_id, issue_id, tool_target, title, prompt, draft_id, design_mode)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&issue_id)
    .bind(&target)
    .bind(&feature_title)
    .bind(&prompt)
    .bind(&draft_id)
    .bind(&mode)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, PrototypePrompt>("SELECT * FROM prototype_prompts WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

fn tool_hint(target: &str) -> &'static str {
    match target {
        "opendesign" => "面向 OpenDesign：强调组件层级、自动布局（auto-layout）、可复用组件与设计 token 的机器可读性。",
        "stitch" => "面向 Google Stitch：强调语义化设计系统、严格排版网格与配色 token、组件状态完整。",
        "claude_design" => "面向 Claude Design：用自然语言完整描述交互流、信息架构与视觉风格，细节充分。",
        _ => "面向通用设计工具：用清晰结构描述界面骨架、组件层级、交互流与可量化的视觉 token。",
    }
}

/// Read the target project's design contract (DESIGN.md or equivalent) so the
/// generated prompt can mirror its structure and detail level.
fn read_repo_design(repo_path: &str) -> Option<String> {
    if repo_path.trim().is_empty() {
        return None;
    }
    for name in [
        "DESIGN.md",
        "design.md",
        "docs/DESIGN.md",
        ".autoforge/docs/DESIGN.md",
        ".autoforge/specs/DESIGN.md",
    ] {
        if let Ok(c) = std::fs::read_to_string(std::path::Path::new(repo_path).join(name)) {
            if !c.trim().is_empty() {
                return Some(c);
            }
        }
    }
    None
}

/// The 10-section design-spec skeleton. Shared by the LLM instruction and the
/// offline heuristic so both produce a `design.md`-grade brief.
const DESIGN_SECTIONS: &str = "\
1. **Overview** — 产品气质、核心设计隐喻、整体布局骨架（一段点明唯一强调色与整体风格基调）。
2. **Colors** — 强调色 / 语义状态色 / 表面层级 / 文本层级 / 描边，每项都给**具体 token**（HEX 或 CSS 变量名）与使用意图；说明深色/浅色与主题切换机制。
3. **Typography** — 字族分工（display / sans / mono）、字号阶梯（列出具体 px）、行高、惯用搭配（页面标题 / KPI 数字 / kicker 标签 / 正文）。
4. **Layout** — 整体骨架（栏宽、固定区域）、间距尺度（以 2/4 为基的具体数值）、栅格规则。
5. **Elevation & Depth** — 阴影层级（每级用途）、focus 态光环、毛玻璃等深度表达。
6. **Shapes** — 圆角尺度（具体 px）、pill、头像/容器形状约定。
7. **Components** — 列出关键组件（按钮及变体 / chip / 卡片面板 / 输入字段 / 分段控件 / 下拉 / 开关 / 头像…），每个给出结构、尺寸与各状态（hover/active/disabled/focus）。
8. **Screens & States** — 针对本需求逐屏展开：页面结构、组件层级、主要交互流，以及空态 / 加载态 / 错误态 / 成功态。
9. **Motion** — 入场、过渡、微交互动效（时长与缓动），并尊重 prefers-reduced-motion。
10. **Do's and Don'ts** — 该设计风格的硬性约束与禁区。";

fn heuristic_prompt(
    project: &str,
    target: &str,
    feature: &str,
    desc: &str,
    design_ctx: &str,
) -> String {
    let mut s = format!(
        "# 设计提示词 · 为产品「{project}」设计「{feature}」\n\n\
         {}\n\n\
         ## 需求\n{desc}\n",
        tool_hint(target)
    );
    if !design_ctx.trim().is_empty() {
        s.push_str(&format!(
            "\n## 现有设计系统（必须严格遵循，保持一致，不得偏离既定风格）\n{design_ctx}\n"
        ));
    } else {
        s.push_str(
            "\n## 现有设计系统\n（项目暂无 DESIGN.md，请在下方自行建立一套完整、自洽的设计体系）\n",
        );
    }
    s.push_str(&format!(
        "\n## 输出要求（详细程度对标 Google Labs design.md，逐节展开、不得省略）\n{DESIGN_SECTIONS}\n\n\
         约束：颜色 / 字号 / 圆角 / 间距 / 阴影都必须给出**具体数值或 token**；信息密度对标 design.md；中文输出。\n"
    ));
    s
}

async fn llm_prompt(
    db: &crate::db::Db,
    project_id: &str,
    project: &str,
    target: &str,
    feature: &str,
    desc: &str,
    design_ctx: &str,
) -> Option<String> {
    let design_block = if design_ctx.trim().is_empty() {
        "（项目暂无设计契约文档，请自行建立一套完整、自洽、可量化的设计体系）".to_string()
    } else {
        design_ctx.to_string()
    };

    let prompt = format!(
        "请为产品「{project}」的「{feature}」生成一份**详尽的设计提示词**，\
         其详细程度与信息密度需对标 Google Labs 的 design.md 规范（含机器可读 token + 设计意图）。\n\n\
         {hint}\n\n\
         ## 需求\n{feature}\n{desc}\n\n\
         ## 现有设计系统参考（必须严格对标其详细程度；若项目已有 DESIGN.md，须延续其风格、token 与命名，不得另起一套）\n{design_block}\n\n\
         ## 输出结构（用 Markdown 逐节展开，每节都要充实，不要省略任何一节）\n{sections}\n\n\
         硬性要求：\n\
         - 颜色、字号、行高、圆角、间距、阴影一律给出**具体数值（px/HEX）或 token / CSS 变量名**，不要含糊形容词。\n\
         - 若提供了 DESIGN.md，直接复用其 token 命名与风格隐喻，保持一致。\n\
         - 针对本需求给出逐屏（Screens & States）的结构、组件层级与交互。\n\
         - 只输出设计提示词本体（Markdown），不要任何前言或结语解释。",
        hint = tool_hint(target),
        sections = DESIGN_SECTIONS,
    );
    // 聚焦召回键：产品 + 功能 + 工具，命中本项目的设计风格/原型经验（而非大段模板）。
    let recall_q = format!("{} {} {} 原型设计", project, feature, target);
    let raw = crate::agents::llm::run_system_role_text(
        db,
        "prototype",
        &prompt,
        Some(
            "你是世界级产品/设计系统专家，精通 Google Labs design.md 规范。\
             你只输出可直接粘贴进设计工具（OpenDesign / Stitch / Claude Design）的完整设计提示词：\
             结构严谨、信息密度高、包含可量化的设计 token，使用中文。",
        ),
        Some(project_id),
        Some(&recall_q),
    )
    .await
    .ok()?;
    if raw.trim().is_empty() {
        None
    } else {
        Some(raw.trim().to_string())
    }
}

// ============================================================================
// OpenDesign 本地服务：可配置「启动命令 + 访问 URL」（存 app_settings），
// 一键拉起本地服务并由前端打开浏览器。命令/URL 来自用户设置，默认见下。
// ============================================================================

/// OpenDesign（nexu-io/open-design）是 pnpm 单仓 monorepo，**没有** `npx opendesign`
/// 这种入口，也不跑在 5173；唯一生命周期入口是 `pnpm tools-dev`。因此默认走「自动模式」：
/// 检测已有检出（无则浅克隆）→ corepack/pnpm install → `tools-dev start web` → 轮询就绪 URL。
/// 仅当用户在设置里显式填了「启动命令」时，才回落到旧的「自定义命令 + URL」逃生通道。
const OPENDESIGN_REPO_URL: &str = "https://github.com/nexu-io/open-design.git";
/// 默认命令留空 = 自动模式；非空 = 自定义模式（逃生通道）。
const OPENDESIGN_DEFAULT_COMMAND: &str = "";
/// 默认 URL 留空：自动模式下 URL 由 `tools-dev status` 解析得到，无需预设端口。
const OPENDESIGN_DEFAULT_URL: &str = "";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDesignSettings {
    /// 自定义启动命令（逃生通道）：非空则按旧行为执行该命令并打开 `url`；
    /// 留空（默认）走自动模式（检测/克隆/安装/启动 nexu-io/open-design）。
    pub command: String,
    /// 服务就绪后要打开的浏览器地址。自动模式下会被实际解析到的 URL 覆盖。
    pub url: String,
    /// 可选：显式指定本地 open-design 检出路径。留空则自动探测常见位置，
    /// 仍找不到时克隆到 AutoForge 自管目录。
    pub repo_path: String,
}

async fn read_setting(state: &AppState, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

async fn write_setting(state: &AppState, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn load_opendesign_settings(state: &AppState) -> OpenDesignSettings {
    let command = read_setting(state, "opendesign.command")
        .await
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OPENDESIGN_DEFAULT_COMMAND.to_string());
    let url = read_setting(state, "opendesign.url")
        .await
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OPENDESIGN_DEFAULT_URL.to_string());
    let repo_path = read_setting(state, "opendesign.repo_path")
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    OpenDesignSettings {
        command,
        url,
        repo_path,
    }
}

#[tauri::command]
pub async fn get_opendesign_settings(
    state: State<'_, AppState>,
) -> Result<OpenDesignSettings, String> {
    Ok(load_opendesign_settings(&state).await)
}

/// 保存 OpenDesign 启动配置。三者均可为空：命令空=自动模式，repo_path 空=自动探测/克隆，
/// URL 空=自动模式下由 `tools-dev status` 解析。
#[tauri::command]
pub async fn set_opendesign_settings(
    command: String,
    url: String,
    repo_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<OpenDesignSettings, String> {
    write_setting(&state, "opendesign.command", command.trim()).await?;
    write_setting(&state, "opendesign.url", url.trim()).await?;
    write_setting(
        &state,
        "opendesign.repo_path",
        repo_path.unwrap_or_default().trim(),
    )
    .await?;
    Ok(load_opendesign_settings(&state).await)
}

/// 简单的 URL 可达性探测（与 dev_server 一致：2s 超时、容忍自签名证书）。
async fn url_reachable(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .danger_accept_invalid_certs(true)
        .build()
    else {
        return false;
    };
    client.get(url).send().await.is_ok()
}

// ---- 日志：自动启动全过程写入临时日志文件，失败时把末尾内嵌进错误，便于排查 ----

fn opendesign_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("autoforge-opendesign.log")
}

/// 截断到日志开头（每次自动启动重置），返回新句柄前先清空旧内容。
fn reset_log() {
    use std::io::Write;
    if let Ok(mut f) = std::fs::File::create(opendesign_log_path()) {
        let _ = writeln!(f, "# OpenDesign 自动启动日志");
    }
}

fn log_line(s: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(opendesign_log_path())
    {
        let _ = writeln!(f, "{s}");
    }
}

fn read_log() -> String {
    std::fs::read_to_string(opendesign_log_path()).unwrap_or_default()
}

/// 按字节安全截取字符串末尾（对齐到 UTF-8 字符边界，避免切片 panic）。
fn tail(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// 在 `cwd` 执行 shell 命令，stdout+stderr 一并落日志并返回组合输出；超时/非零退出返回 Err。
async fn run_capture(
    label: &str,
    script: &str,
    cwd: &std::path::Path,
    timeout_secs: u64,
) -> Result<String, String> {
    log_line(&format!("\n$ [{label}] {script}\n# cwd: {}", cwd.display()));
    let mut cmd = crate::core::platform::shell(script);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out =
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output()).await
        {
            Err(_) => {
                let m = format!("命令超时（>{timeout_secs}s）：{label}");
                log_line(&m);
                return Err(m);
            }
            Ok(Err(e)) => {
                let m = format!("命令无法启动：{label}：{e}");
                log_line(&m);
                return Err(m);
            }
            Ok(Ok(o)) => o,
        };
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    log_line(&combined);
    if out.status.success() {
        Ok(combined)
    } else {
        Err(format!(
            "{label} 失败（退出码 {}）：\n{}",
            out.status.code().unwrap_or(-1),
            tail(&combined, 1500)
        ))
    }
}

/// 解析可用的 pnpm 调用前缀：优先直接 `pnpm`，否则借 Node 自带的 `corepack pnpm`
/// （corepack 会按仓库 packageManager 字段自动选定锁定的 pnpm 版本）。
fn resolve_pnpm() -> Result<String, String> {
    if crate::core::platform::has_executable("pnpm") {
        Ok("pnpm".to_string())
    } else if crate::core::platform::has_executable("corepack") {
        Ok("corepack pnpm".to_string())
    } else {
        Err("未找到 pnpm，也没有 corepack。请安装 Node 24（自带 corepack），或先运行 `corepack enable`。"
            .to_string())
    }
}

/// 目录是否为 open-design 检出：根 package.json 的 name == "open-design"。
fn is_opendesign_checkout(dir: &std::path::Path) -> bool {
    let Ok(txt) = std::fs::read_to_string(dir.join("package.json")) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
        return false;
    };
    v.get("name").and_then(|n| n.as_str()) == Some("open-design")
}

/// 常见的本地检出候选位置（复用，避免重复克隆大仓库）。
fn candidate_checkouts() -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Some(home) = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
    {
        let base = std::path::PathBuf::from(home);
        for sub in ["projects", "code", "src", "dev", "workspace", "repos", "."] {
            out.push(base.join(sub).join("open-design"));
        }
    }
    out
}

/// 浅克隆 open-design 到目标目录。
async fn clone_opendesign(dest: &std::path::Path) -> Result<(), String> {
    if !crate::core::platform::has_executable("git") {
        return Err("未找到 git，无法克隆 OpenDesign。请安装 git，或在「设置」里指定本地检出路径。"
            .to_string());
    }
    let parent = dest.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    // 已存在但非有效检出 → 先移除，避免 git clone 因目录非空失败。
    if dest.exists() {
        let _ = std::fs::remove_dir_all(dest);
    }
    let script = format!(
        "git clone --depth 1 {} \"{}\"",
        OPENDESIGN_REPO_URL,
        dest.to_string_lossy()
    );
    run_capture("git clone", &script, parent, 600).await?;
    Ok(())
}

/// 解析最终使用的检出路径：显式设置 > 探测已有 > 自管目录已克隆 > 克隆。
async fn resolve_repo_path(cfg: &OpenDesignSettings) -> Result<std::path::PathBuf, String> {
    let explicit = cfg.repo_path.trim();
    if !explicit.is_empty() {
        let p = std::path::PathBuf::from(explicit);
        if is_opendesign_checkout(&p) {
            return Ok(p);
        }
        return Err(format!(
            "设置中的 OpenDesign 路径不是有效检出（缺 package.json 或 name≠open-design）：{}",
            p.display()
        ));
    }
    for cand in candidate_checkouts() {
        if is_opendesign_checkout(&cand) {
            log_line(&format!("复用已有检出：{}", cand.display()));
            return Ok(cand);
        }
    }
    let managed = std::path::PathBuf::from(crate::state::opendesign_base());
    if is_opendesign_checkout(&managed) {
        log_line(&format!("复用自管检出：{}", managed.display()));
        return Ok(managed);
    }
    log_line(&format!("未找到本地检出 → 克隆到 {}", managed.display()));
    clone_opendesign(&managed).await?;
    Ok(managed)
}

/// 首次安装依赖（node_modules 不存在时）。
async fn ensure_installed(repo: &std::path::Path, pnpm: &str) -> Result<(), String> {
    if repo.join("node_modules").is_dir() {
        return Ok(());
    }
    log_line("node_modules 不存在 → 执行 pnpm install（首次较慢）");
    if crate::core::platform::has_executable("corepack") {
        // best-effort：启用仓库锁定的 pnpm 版本，失败不阻断（pnpm 可能已全局可用）。
        let _ = run_capture("corepack enable", "corepack enable", repo, 60).await;
    }
    run_capture("pnpm install", &format!("{pnpm} install"), repo, 900).await?;
    Ok(())
}

/// 查询 `tools-dev status --json`，若 web 已在运行则返回其 URL。
async fn web_url_if_running(repo: &std::path::Path, pnpm: &str) -> Option<String> {
    let mut cmd = crate::core::platform::shell(&format!("{pnpm} tools-dev status --json"));
    cmd.current_dir(repo)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .ok()?
        .ok()?;
    let so = String::from_utf8_lossy(&out.stdout);
    // 输出可能夹带 pnpm 脚本头，截取首个 '{' 到末个 '}' 之间的 JSON 对象。
    let start = so.find('{')?;
    let end = so.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&so[start..=end]).ok()?;
    let url = v.get("apps")?.get("web")?.get("url")?.as_str()?.to_string();
    (!url.is_empty()).then_some(url)
}

/// 自动模式：检测/克隆 → 安装 → 启动 web → 轮询就绪 URL。
async fn launch_opendesign_auto(state: &AppState, cfg: &OpenDesignSettings) -> Result<String, String> {
    reset_log();
    if !crate::core::platform::has_executable("node") {
        return Err("未检测到 Node.js。OpenDesign 需要 Node 24 + pnpm 10.33（建议用 nvm/fnm：`nvm install 24 && nvm use 24`），随后重试。"
            .to_string());
    }
    let pnpm = resolve_pnpm()?;
    let repo = resolve_repo_path(cfg).await?;
    log_line(&format!("OpenDesign 检出：{}", repo.display()));
    ensure_installed(&repo, &pnpm).await?;

    // 已在运行？直接复用其 URL（避免重复启动）。
    if let Some(url) = web_url_if_running(&repo, &pnpm).await {
        if url_reachable(&url).await {
            log_line(&format!("web 已在运行：{url}"));
            let _ = write_setting(state, "opendesign.url", &url).await;
            return Ok(url);
        }
    }

    // 后台启动 web（`start` 拉起常驻服务后即返回，不像 `run` 前台阻塞）。
    run_capture(
        "tools-dev start web",
        &format!("{pnpm} tools-dev start web"),
        &repo,
        180,
    )
    .await?;

    // 轮询直到 status 给出可达 URL（首次可能要编译，~180s）。
    for _ in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Some(url) = web_url_if_running(&repo, &pnpm).await {
            if url_reachable(&url).await {
                log_line(&format!("就绪：{url}"));
                let _ = write_setting(state, "opendesign.url", &url).await;
                return Ok(url);
            }
        }
    }
    Err(format!(
        "OpenDesign 启动超时（web 未就绪）。日志末尾：\n{}",
        tail(&read_log(), 1500)
    ))
}

/// 自定义模式（逃生通道）：执行用户填写的命令并轮询其 URL，沿用旧行为。
async fn launch_opendesign_custom(cfg: &OpenDesignSettings) -> Result<String, String> {
    let url = cfg.url.trim().to_string();
    if url.is_empty() {
        return Err("自定义启动命令模式下必须填写「访问 URL」。".to_string());
    }
    if url_reachable(&url).await {
        return Ok(url);
    }
    // detached：让本地服务独立于桌面壳进程组存活，退出 AutoForge 不连带杀掉它。
    let mut cmd = crate::core::platform::shell(&cfg.command);
    crate::core::platform::detach_process_group(&mut cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("启动 OpenDesign 失败：{e}（命令：{}）", cfg.command))?;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if url_reachable(&url).await {
            return Ok(url);
        }
    }
    Ok(url)
}

/// 拉起本地 OpenDesign 服务并返回要打开的 URL（浏览器打开交给前端 `open_url`）。
///
/// - 未配置自定义命令（默认）→ 自动模式：检测已有检出/克隆 → install → `tools-dev start web`。
/// - 配置了自定义命令 → 自定义模式：执行该命令并轮询 `url`。
#[tauri::command]
pub async fn launch_opendesign(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = load_opendesign_settings(&state).await;
    if cfg.command.trim().is_empty() {
        launch_opendesign_auto(&state, &cfg).await
    } else {
        launch_opendesign_custom(&cfg).await
    }
}

/// 返回自动启动日志末尾（前端用于排查「拉不起来」时展示原因）。
#[tauri::command]
pub async fn get_opendesign_log() -> Result<String, String> {
    Ok(tail(&read_log(), 16000))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P4 文档源汇总：需求 PRD（有 draft 时默认选）+ 技术规格按分类聚合（architecture/api 默认选）。
    /// design_md 走文件（测试 repo_path 不存在 → 跳过，不影响 DB 源验证）。
    #[tokio::test]
    async fn collect_doc_sources_aggregates_prd_and_specs() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE project_specs (id TEXT PRIMARY KEY, project_id TEXT, category TEXT, title TEXT)")
            .execute(&db).await.unwrap();
        sqlx::query("CREATE TABLE blueprint_drafts (id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', prd_markdown TEXT NOT NULL DEFAULT '')")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO project_specs VALUES ('s1','p1','architecture','架构'),('s2','p1','architecture','架构2'),('s3','p1','testing','测试')")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO blueprint_drafts (id, title, prd_markdown) VALUES ('d1','登录改造','# PRD 内容')")
            .execute(&db).await.unwrap();

        let out = collect_doc_sources(&db, "p1", "/nonexistent-repo", Some("d1")).await;
        // 需求 PRD 在（默认选）。
        let prd = out.iter().find(|d| d.kind == "blueprint_prd").expect("有 PRD 源");
        assert!(prd.default_on && prd.title.contains("登录改造"));
        // 规格按分类聚合：architecture 一条（去重）+ testing 一条。
        let specs: Vec<&DocSource> = out.iter().filter(|d| d.kind == "spec").collect();
        assert_eq!(specs.len(), 2, "两个分类各一条聚合项");
        let arch = specs.iter().find(|d| d.r#ref == "architecture").unwrap();
        assert!(arch.default_on, "architecture 默认选中");
        let testing = specs.iter().find(|d| d.r#ref == "testing").unwrap();
        assert!(!testing.default_on, "testing 默认不选");
    }

    /// P4 §3.5C：read_doc_refs 按选中 ref 读正文拼上下文（PRD + spec 分类聚合），跳未知 ref。
    #[tokio::test]
    async fn read_doc_refs_assembles_selected_docs() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE blueprint_drafts (id TEXT PRIMARY KEY, prd_markdown TEXT NOT NULL DEFAULT '')")
            .execute(&db).await.unwrap();
        sqlx::query("CREATE TABLE project_specs (id TEXT PRIMARY KEY, project_id TEXT, category TEXT, content TEXT)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO blueprint_drafts VALUES ('d1','# 登录页 PRD 正文')").execute(&db).await.unwrap();
        sqlx::query("INSERT INTO project_specs VALUES ('s1','p1','api','接口约束A'),('s2','p1','api','接口约束B')")
            .execute(&db).await.unwrap();

        let refs = vec![
            "blueprint_prd:d1".to_string(),
            "spec:api".to_string(),
            "workspace:未知".to_string(), // 未知 ref → 跳过
        ];
        let ctx = read_doc_refs(&db, "/nonexistent", "p1", &refs).await;
        assert!(ctx.contains("# 需求文档(PRD)") && ctx.contains("登录页 PRD 正文"));
        assert!(ctx.contains("# 技术规格·api") && ctx.contains("接口约束A") && ctx.contains("接口约束B"));
    }

    /// 原型按需求归档：draft_id 列 + 按 draft_id 过滤（同项目不同需求各自独立列出）。
    #[tokio::test]
    async fn prototypes_filter_by_draft() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // 与迁移 0021+0083 同构的最小建表（含 draft_id/design_mode）。
        sqlx::query(
            "CREATE TABLE prototype_prompts (id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
             issue_id TEXT, tool_target TEXT NOT NULL DEFAULT 'generic', title TEXT NOT NULL DEFAULT '',
             prompt TEXT NOT NULL DEFAULT '', draft_id TEXT NOT NULL DEFAULT '',
             design_mode TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO prototype_prompts (id, project_id, draft_id, design_mode, title) VALUES
               ('a','p1','d1','new','登录页'),
               ('b','p1','d1','existing','登录页改版'),
               ('c','p1','d2','new','结算页')",
        )
        .execute(&db)
        .await
        .unwrap();

        // 按需求 d1 过滤 → 2 条；d2 → 1 条。
        let d1 = sqlx::query_as::<_, PrototypePrompt>("SELECT * FROM prototype_prompts WHERE draft_id=? ORDER BY id")
            .bind("d1").fetch_all(&db).await.unwrap();
        assert_eq!(d1.len(), 2, "需求 d1 有两个原型");
        assert_eq!(d1[1].design_mode, "existing", "design_mode 落库可读");
        let d2 = sqlx::query_as::<_, PrototypePrompt>("SELECT * FROM prototype_prompts WHERE draft_id=?")
            .bind("d2").fetch_all(&db).await.unwrap();
        assert_eq!(d2.len(), 1);
        // 项目级(不带 draft)仍列全部 3 条。
        let all = sqlx::query_as::<_, PrototypePrompt>("SELECT * FROM prototype_prompts WHERE project_id=?")
            .bind("p1").fetch_all(&db).await.unwrap();
        assert_eq!(all.len(), 3);
    }
}
