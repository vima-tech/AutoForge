use crate::models::issue::Issue;
use crate::models::project::Project;
use crate::models::prototype::PrototypePrompt;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
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

// ============================================================================
// OpenDesign 本地服务：可配置「启动命令 + 访问 URL」（存 app_settings），
// 一键拉起本地服务并由前端打开浏览器。命令/URL 来自用户设置，默认见下。
// ============================================================================

const OPENDESIGN_DEFAULT_COMMAND: &str = "npx opendesign";
const OPENDESIGN_DEFAULT_URL: &str = "http://localhost:5173";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDesignSettings {
    /// 拉起本地 OpenDesign 服务的 shell 命令（为空表示「不启动、仅打开 URL」）。
    pub command: String,
    /// 服务就绪后要打开的浏览器地址。
    pub url: String,
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
    OpenDesignSettings { command, url }
}

#[tauri::command]
pub async fn get_opendesign_settings(
    state: State<'_, AppState>,
) -> Result<OpenDesignSettings, String> {
    Ok(load_opendesign_settings(&state).await)
}

/// 保存 OpenDesign 启动配置。命令可为空（仅打开 URL）；URL 为空时回落到默认。
#[tauri::command]
pub async fn set_opendesign_settings(
    command: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<OpenDesignSettings, String> {
    let url = {
        let t = url.trim();
        if t.is_empty() { OPENDESIGN_DEFAULT_URL } else { t }
    };
    write_setting(&state, "opendesign.command", command.trim()).await?;
    write_setting(&state, "opendesign.url", url).await?;
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

/// 拉起本地 OpenDesign 服务并返回要打开的 URL（浏览器打开交给前端 `open_url`，
/// 保持本命令不依赖 Tauri opener）。
///
/// 行为：
/// 1. 若 URL 已可达 → 直接返回（服务已在运行，避免重复启动）。
/// 2. 否则执行配置的启动命令（detached 进程组，独立于桌面壳存活），
///    再轮询等待服务就绪（最多 ~30s）。
/// 3. 命令为空且 URL 不可达 → 报错提示去设置里配置启动命令。
#[tauri::command]
pub async fn launch_opendesign(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = load_opendesign_settings(&state).await;

    if url_reachable(&cfg.url).await {
        return Ok(cfg.url);
    }

    if cfg.command.is_empty() {
        return Err(format!(
            "OpenDesign 服务未运行（{}），且未配置启动命令。请在「设置」中填写启动命令。",
            cfg.url
        ));
    }

    // detached：让本地服务独立于桌面壳进程组存活，关闭/退出 AutoForge 不连带杀掉它。
    let mut cmd = crate::core::platform::shell(&cfg.command);
    crate::core::platform::detach_process_group(&mut cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _child = cmd
        .spawn()
        .map_err(|e| format!("启动 OpenDesign 失败：{e}（命令：{}）", cfg.command))?;
    // 不持有 child 句柄：进程已 detach 成独立进程组，由用户自行管理生命周期。

    // 轮询等待就绪（每 1s 探一次，最多 30 次）。
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if url_reachable(&cfg.url).await {
            return Ok(cfg.url);
        }
    }

    // 超时仍未就绪：返回 URL 让前端照常打开（服务可能仍在编译/慢启动）。
    Ok(cfg.url)
}
