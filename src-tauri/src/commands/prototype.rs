use crate::models::issue::Issue;
use crate::models::project::Project;
use crate::models::prototype::PrototypePrompt;
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_prototype_prompts(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PrototypePrompt>, String> {
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
    state: State<'_, AppState>,
) -> Result<PrototypePrompt, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project_id))?;

    let issue: Option<Issue> = if let Some(iid) = &issue_id {
        sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(iid)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
    } else {
        None
    };

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

    let target = tool_target.unwrap_or_else(|| "generic".to_string());
    let (feature_title, feature_desc) = match &issue {
        Some(i) => (i.title.clone(), i.description.clone()),
        None => (
            format!("{} 产品界面", project.name),
            project.description.clone(),
        ),
    };

    let heuristic = heuristic_prompt(&project.name, &target, &feature_title, &feature_desc, &design_ctx);
    let prompt = llm_prompt(&state.db, &project.id, &project.name, &target, &feature_title, &feature_desc, &design_ctx)
        .await
        .unwrap_or(heuristic);

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO prototype_prompts (id, project_id, issue_id, tool_target, title, prompt)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&issue_id)
    .bind(&target)
    .bind(&feature_title)
    .bind(&prompt)
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
