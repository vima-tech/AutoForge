use crate::core::event;
use crate::models::agent::Agent;
use crate::models::conversation::{Conversation, ConversationAttachment, Message};
use crate::models::orchestration::{ConversationTask, StartConversationTask};
use crate::state::AppState;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, State};
use tracing::{info, warn};
use uuid::Uuid;

// 会议室「立即编码」草稿：AI 梳理讨论后的结构化需求描述
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingBrief {
    pub title: String,
    pub functional_points: Vec<String>,
    pub involved_modules: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub requirement_type: String,
    pub risk_level: String,
    pub raw_text: String, // 保留原始文本供编辑
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationPlan {
    steps: Vec<ConversationPlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationPlanStep {
    #[serde(rename = "type")]
    step_type: String,
    agents: Vec<String>,
    instruction: String,
}

#[derive(Debug, Clone)]
struct AgentOutcome {
    agent_id: String,
    agent_name: String,
    ok: bool,
    text: String,
}

/// 系统角色 Agent 执行入参聚合体。
/// 把 run_system_agent_markdown / run_system_agent_text 共用的一组参数收拢成结构体，
/// 避免函数签名参数过多（clippy::too_many_arguments）。字段均为借用，生命周期统一为 'a。
#[derive(Debug, Clone)]
struct SystemAgentParams<'a> {
    conversation_id: &'a str,
    agent: &'a Agent,
    kind: &'a str,
    prompt: &'a str,
    fallback_system_prompt: &'a str,
    project_id: Option<&'a str>,
    recall_key: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactPayload {
    kind: String,
    title: String,
    #[serde(default)]
    rows: Vec<[String; 2]>,
    body: String,
}

#[tauri::command]
pub async fn start_conversation_task(
    payload: StartConversationTask,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ConversationTask, String> {
    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&payload.conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", payload.conversation_id))?;

    let trigger_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM messages WHERE id=? AND conversation_id=?")
            .bind(&payload.trigger_message_id)
            .bind(&payload.conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    if trigger_exists.is_none() {
        return Err("触发消息不存在或不属于当前对话".to_string());
    }

    // 会话串行锁：把「检查是否已有运行中任务 + 插入新任务」做成对并发调用原子的临界区。
    // 否则两条几乎同时到达的消息（或前端重试）会各自插入一条 running 任务，同一会话并发跑多个
    // 任务、互相覆盖消息流。锁内拒绝重复，保证同一会话任一时刻至多一个 running 任务。
    let lock = crate::state::conversation_lock(&payload.conversation_id);
    let _guard = lock.lock().await;

    let already_running: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM conversation_tasks WHERE conversation_id=? AND status='running' LIMIT 1",
    )
    .bind(&payload.conversation_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if already_running.is_some() {
        return Err("当前会话已有正在运行的任务，请等待其完成后再试".to_string());
    }

    let task_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO conversation_tasks
         (id, conversation_id, trigger_message_id, instruction, status, started_at)
         VALUES (?, ?, ?, ?, 'running', datetime('now'))",
    )
    .bind(&task_id)
    .bind(&payload.conversation_id)
    .bind(&payload.trigger_message_id)
    .bind(&payload.instruction)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let task = sqlx::query_as::<_, ConversationTask>("SELECT * FROM conversation_tasks WHERE id=?")
        .bind(&task_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let db = state.db.clone();
    let app_for_task = app.clone();
    // trace 关联标签：本次任务的所有 LLM/工具 span 都带上会议室/任务/项目，便于按条件筛选。
    // task_local 不跨 spawn，故在 spawn 的任务体外层包一层 with_tags。
    let trace_tags = crate::core::trace::TraceTags {
        conversation_id: Some(payload.conversation_id.clone()),
        task_id: Some(task_id.clone()),
        project_id: conversation.project_id.clone(),
        ..Default::default()
    };
    tauri::async_runtime::spawn(crate::core::trace::with_tags(trace_tags, async move {
        if let Err(e) = execute_conversation_task(
            db.clone(),
            app_for_task.clone(),
            conversation,
            payload.clone(),
            task_id.clone(),
        )
        .await
        {
            warn!("[orchestration] task {} failed: {}", task_id, e);
            let _ = sqlx::query(
                "UPDATE conversation_tasks
                 SET status='failed', error=?, completed_at=datetime('now')
                 WHERE id=?",
            )
            .bind(&e)
            .bind(&task_id)
            .execute(&db)
            .await;
            emit_task_update(&app_for_task, &payload.conversation_id, &task_id, "failed");
        }
    }));

    Ok(task)
}

/// 会议室「立即编码」第一步（AI 起草）：把一次会议室讨论 + 项目上下文，用一次轻量 LLM
/// pass 梳理成一份**可直接交给编码 AI 执行的工单草稿**（标题 + 功能点要点 + 范围/约束/验收）。
/// 返回纯文本草稿，由前端填进确认弹窗供操作者编辑——草稿是辅助而非闸门，操作者确认才进入编码。
#[tauri::command]
pub async fn draft_coding_brief(
    conversation_id: String,
    window_size: Option<i64>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;
    if conversation.project_id.is_none() {
        return Err("仅绑定项目的群聊可使用「立即编码」".to_string());
    }

    let window = window_size.unwrap_or(30).clamp(5, 100);
    let context = assemble_conversation_context(&state.db, &conversation_id, window).await?;
    if context.trim().is_empty() {
        return Err("当前会议室没有可梳理的讨论内容".to_string());
    }
    let agent = load_brief_agent(&state.db).await?;

    let prompt = build_brief_prompt(&context);

    crate::agents::llm::run_agent_text(&state.db, &agent, &prompt, None, &[])
        .await
        .map_err(|e| e.to_string())
}

/// 构造「立即编码」需求梳理的提示词。三个入口（非流式 / 详细 / 流式）共用一份，
/// 保证不同路径产出的工单结构完全一致。
fn build_brief_prompt(context: &str) -> String {
    format!(
        "下面是一段会议室讨论与项目上下文。请把其中达成的开发意图梳理成一份**清晰、可直接交给\
         编码 AI 执行的需求工单**。\n\n\
         ## 输出格式\n\
         严格按以下结构输出，不要省略任何环节：\n\n\
         **标题：** <一句话需求标题，简洁有力>\n\n\
         **功能点与需求：**\n\
         - <功能点 1：用户故事或具体任务>\n\
         - <功能点 2>\n\
         - （如需多项功能，继续列举）\n\n\
         **涉及的模块与文件：**\n\
         根据讨论推断可能需要修改的关键模块/文件路径（如 src/pages/Dashboard.tsx, src-tauri/src/commands/issues.rs 等）。\n\
         - <模块/文件 1>\n\
         - <模块/文件 2>\n\
         - （列出最可能受影响的 3-5 个关键点）\n\n\
         **关键约束与技术考量：**\n\
         - 列举讨论中提及的设计约束、兼容性需求、性能要求等\n\n\
         **验收要点：**\n\
         - 描述如何判断功能已正确实现\n\n\
         **需求类型：** <功能新增 | 功能改进 | Bug修复 | 重构优化 | 其他>\n\
         **初步风险等级：** <低 | 中 | 高>\n\n\
         ## 输出原则\n\
         1. 只保留与「本次要编码的需求」直接相关的内容，剔除闲聊与已废弃的设想。\n\
         2. 推断（而非直译）讨论意图——从对话中读出隐含的功能需求。\n\
         3. 涉及的文件与模块要基于讨论背景与项目结构推测，帮助编码 AI 快速定位。\n\
         4. 用中文，简洁明确，直接给工单——不要反问、不要寒暄、不要解释你在做什么。\n\n\
         ## 讨论与项目上下文\n{context}"
    )
}

/// 从标记文本中提取 markdown 列表项
fn extract_list_items(text: &str, start_marker: &str, end_marker: Option<&str>) -> Vec<String> {
    let mut items = Vec::new();
    if let Some(start) = text.find(start_marker) {
        let after = &text[start + start_marker.len()..];
        let end = if let Some(m) = end_marker {
            after.find(m).unwrap_or(after.len())
        } else {
            after.len()
        };
        let section = &after[..end];
        for line in section.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('-') || trimmed.starts_with('•') || trimmed.starts_with('*') {
                let item = trimmed[1..].trim().to_string();
                if !item.is_empty() && !item.starts_with('（') {
                    items.push(item);
                }
            }
        }
    }
    items
}

/// 提取简单的字段值（用于「字段：值」格式）
fn extract_field(text: &str, field_name: &str) -> String {
    let patterns = vec![
        format!("**{}[：:] **", field_name),
        format!("{}[：:]", field_name),
        format!("**{}**[：:]", field_name),
    ];
    for pattern in patterns {
        for line in text.lines() {
            if line.contains(&pattern.replace("[：:]", ":")) || line.contains(&pattern.replace("[：:]", "：")) {
                if let Some(colon_pos) = line.find(':').or_else(|| line.find('：')) {
                    let value = line[colon_pos + 1..].trim().replace("**", "").trim().to_string();
                    if !value.is_empty() {
                        return value;
                    }
                }
            }
        }
    }
    String::new()
}

/// 根据需求的特征精化风险评估：考虑涉及的模块数、关键字、复杂度等
fn refine_risk_assessment(brief: &CodingBrief) -> String {
    let mut risk_score: i32 = 0;

    // 1. 涉及模块数：超过 5 个模块为高风险
    if brief.involved_modules.len() > 5 {
        risk_score += 3;
    } else if brief.involved_modules.len() > 2 {
        risk_score += 2;
    }

    // 2. 功能点数：超过 5 个为高风险
    if brief.functional_points.len() > 5 {
        risk_score += 2;
    }

    // 3. 约束数：多个约束可能增加复杂度
    if brief.constraints.len() > 3 {
        risk_score += 1;
    }

    // 4. 关键字检测：某些操作词暗示高风险
    let content = format!(
        "{} {} {} {} {}",
        brief.title,
        brief.functional_points.join(" "),
        brief.involved_modules.join(" "),
        brief.constraints.join(" "),
        brief.requirement_type
    );

    let high_risk_keywords = vec![
        "重构", "迁移", "删除", "大规模", "底层", "API", "权限",
        "安全", "性能优化", "并发", "分布式", "数据库", "核心",
    ];
    for keyword in high_risk_keywords {
        if content.contains(keyword) {
            risk_score += 2;
            break; // 避免重复计分
        }
    }

    let medium_risk_keywords = vec!["修改", "改进", "重新设计", "兼容"];
    for keyword in medium_risk_keywords {
        if content.contains(keyword) {
            risk_score += 1;
            break;
        }
    }

    // 5. 需求类型：修复通常低风险，新增中等风险，重构高风险
    match brief.requirement_type.as_str() {
        "重构优化" => risk_score += 2,
        "Bug修复" => risk_score = risk_score.saturating_sub(1),
        "功能新增" => risk_score += 1,
        _ => {}
    }

    // 计算最终风险等级
    if risk_score >= 5 {
        "高".to_string()
    } else if risk_score >= 3 {
        "中".to_string()
    } else {
        "低".to_string()
    }
}

/// 会议室「立即编码」详细版（内部用）：返回结构化的代码草稿，前端用于展示预览信息。
fn parse_coding_brief(text: &str) -> CodingBrief {
    let mut brief = CodingBrief {
        title: String::new(),
        functional_points: Vec::new(),
        involved_modules: Vec::new(),
        constraints: Vec::new(),
        acceptance_criteria: Vec::new(),
        requirement_type: "其他".to_string(),
        risk_level: "中".to_string(),
        raw_text: text.to_string(),
    };

    // 提取标题
    for line in text.lines() {
        if line.contains("标题") && (line.contains("：") || line.contains(":")) {
            if let Some(colon_pos) = line.find(':').or_else(|| line.find('：')) {
                let title = line[colon_pos + 1..].trim().replace("**", "").trim().to_string();
                if !title.is_empty() {
                    brief.title = title;
                    break;
                }
            }
        }
    }

    // 提取功能点
    brief.functional_points = extract_list_items(text, "功能点与需求", Some("涉及的模块"));
    if brief.functional_points.is_empty() {
        brief.functional_points = extract_list_items(text, "**功能点与需求**", Some("**涉及"));
    }

    // 提取涉及的模块与文件
    brief.involved_modules = extract_list_items(text, "涉及的模块与文件", Some("关键约束"));
    if brief.involved_modules.is_empty() {
        brief.involved_modules = extract_list_items(text, "相关文件", Some("约束"));
    }

    // 提取约束和技术考量
    brief.constraints = extract_list_items(text, "关键约束与技术考量", Some("验收要点"));
    if brief.constraints.is_empty() {
        brief.constraints = extract_list_items(text, "技术考量", Some("验收"));
    }

    // 提取验收要点
    brief.acceptance_criteria = extract_list_items(text, "验收要点", Some("需求类型"));
    if brief.acceptance_criteria.is_empty() {
        brief.acceptance_criteria = extract_list_items(text, "验收标准", Some("需求"));
    }

    // 提取需求类型
    brief.requirement_type = extract_field(text, "需求类型");
    if brief.requirement_type.is_empty() {
        brief.requirement_type = "其他".to_string();
    }

    // 提取风险等级
    brief.risk_level = extract_field(text, "初步风险等级");
    if brief.risk_level.is_empty() {
        brief.risk_level = extract_field(text, "风险等级");
    }
    if brief.risk_level.is_empty() {
        brief.risk_level = "中".to_string();
    }

    brief
}

/// 会议室「立即编码」详细版：返回结构化代码草稿，含标题、功能点、文件范围、风险等级等。
/// 前端用这些数据展示预览信息，让操作者清晰了解背景。
#[tauri::command]
pub async fn draft_coding_brief_detailed(
    conversation_id: String,
    window_size: Option<i64>,
    state: State<'_, AppState>,
) -> Result<CodingBrief, String> {
    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;
    if conversation.project_id.is_none() {
        return Err("仅绑定项目的群聊可使用「立即编码」".to_string());
    }

    let window = window_size.unwrap_or(30).clamp(5, 100);
    let context = assemble_conversation_context(&state.db, &conversation_id, window).await?;
    if context.trim().is_empty() {
        return Err("当前会议室没有可梳理的讨论内容".to_string());
    }
    let agent = load_brief_agent(&state.db).await?;
    let prompt = build_brief_prompt(&context);

    let text = crate::agents::llm::run_agent_text(&state.db, &agent, &prompt, None, &[])
        .await
        .map_err(|e| e.to_string())?;

    let mut brief = parse_coding_brief(&text);
    // 根据需求特征精化风险评估
    brief.risk_level = refine_risk_assessment(&brief);
    Ok(brief)
}

/// 会议室「立即编码」流式版：边生成边把 AI 的思考增量通过 `CodingBriefChunk` 事件推给前端，
/// 消除「干等」的等待感；生成结束后解析为结构化 `CodingBrief` 返回。前端订阅事件实时滚动日志，
/// promise resolve 时拿到结构化结果填表。
#[tauri::command]
pub async fn draft_coding_brief_stream(
    conversation_id: String,
    window_size: Option<i64>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodingBrief, String> {
    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;
    if conversation.project_id.is_none() {
        return Err("仅绑定项目的群聊可使用「立即编码」".to_string());
    }

    let window = window_size.unwrap_or(30).clamp(5, 100);
    let context = assemble_conversation_context(&state.db, &conversation_id, window).await?;
    if context.trim().is_empty() {
        return Err("当前会议室没有可梳理的讨论内容".to_string());
    }
    let agent = load_brief_agent(&state.db).await?;
    let prompt = build_brief_prompt(&context);

    // 每段增量转成 CodingBriefChunk 事件发射。闭包只捕获 AppHandle + conversation_id，
    // 业务逻辑（llm.rs）仍对 Tauri 无感知——事件出口唯一走 event::emit。
    let app_for_chunk = app.clone();
    let conv_for_chunk = conversation_id.clone();
    let mut on_chunk = move |chunk: &str| {
        event::emit(
            &app_for_chunk,
            event::AppEvent::CodingBriefChunk {
                conversation_id: conv_for_chunk.clone(),
                chunk: chunk.to_string(),
            },
        );
    };

    let text = crate::agents::llm::run_agent_text_streaming(
        &state.db,
        &agent,
        &prompt,
        None,
        &mut on_chunk,
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut brief = parse_coding_brief(&text);
    brief.risk_level = refine_risk_assessment(&brief);
    Ok(brief)
}

/// 会议室「立即编码」入参：操作者在确认弹窗里给出标题 + 功能点工单（可由 `draft_coding_brief`
/// 起草后编辑），可选会话窗口大小与操作者标识。
#[derive(Debug, Clone, Deserialize)]
pub struct StartConversationCoding {
    pub conversation_id: String,
    pub title: String,
    pub brief: String,
    pub window_size: Option<i64>,
    pub admin_id: Option<String>,
    /// 前端生成的 UUID，用于幂等去重；重复请求命中时直接返回已有 CR。
    pub client_request_id: Option<String>,
}

/// 会议室「立即编码」第二步（直奔编码）：依据操作者确认的功能点工单，**自动创建需求 → 建 CR →
/// 入队编码执行**，跳过需求审核（review_1）队列——操作者在会议室点「立即编码」即视为需求侧的
/// 人工决策；代码审核（review_2）仍是合并前的唯一闸门，架构「双审核 / 合并唯一入口」不被破坏。
/// 会话快照 + 项目上下文作为 `work_context` 随 CR 落库，注入编码工单的「需求来源」段。
#[tauri::command]
pub async fn start_conversation_coding(
    payload: StartConversationCoding,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::models::change_request::ChangeRequest, String> {
    let db = &state.db;
    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&payload.conversation_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", payload.conversation_id))?;
    let project_id = conversation
        .project_id
        .clone()
        .ok_or_else(|| "仅绑定项目的群聊可使用「立即编码」".to_string())?;
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&project_id)
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("project {} not found", project_id))?;

    let title = payload.title.trim().to_string();
    let brief = payload.brief.trim().to_string();
    if title.is_empty() || brief.is_empty() {
        return Err("需求标题与功能点不能为空".to_string());
    }
    // 工单内容会进编码 agent 工单，按安全规则过注入消毒（操作者输入也可能来自被污染的讨论）。
    if crate::core::security::has_obvious_injection(&title)
        || crate::core::security::has_obvious_injection(&brief)
    {
        return Err("内容包含可疑指令，已拦截".to_string());
    }

    // 组装会话 + 项目上下文作为编码背景（best-effort：失败不阻断，工单本身已足够编码）。
    let window = payload.window_size.unwrap_or(30).clamp(5, 100);
    let work_context = match assemble_conversation_context(db, &payload.conversation_id, window).await
    {
        Ok(c) if !c.trim().is_empty() => Some(c),
        _ => None,
    };

    let admin_id = payload.admin_id.as_deref().unwrap_or("admin").to_string();

    // 会话串行锁：把「幂等查重 → 建 Issue → 建 CR」整段做成对并发原子的临界区。
    // 否则操作者双击/前端重试会让两次调用都越过下面的查重（彼此都还没插入），各建一条
    // Issue+CR（重复编码同一需求）。Level-2（未传 client_request_id）尤其只有 check-then-insert
    // 保护，无 DB 唯一约束兜底，必须靠此锁关闭 TOCTOU 窗口。
    let lock = crate::state::conversation_lock(&payload.conversation_id);
    let _guard = lock.lock().await;

    // ── 幂等去重 ──────────────────────────────────────────────────────────────
    // Level 1：client_request_id 精确匹配（前端 UUID，最强保证）。
    // Level 2：同 project + title + source_type='conversation' + status='pending_execution' 近似兜底。
    // 命中时直接返回既有 CR；若 Issue 存在但 CR 尚无（前次半途失败），补建 CR 后返回。
    let maybe_existing: Option<crate::models::issue::Issue> =
        if let Some(ref req_id) = payload.client_request_id {
            sqlx::query_as::<_, crate::models::issue::Issue>(
                "SELECT * FROM issues WHERE client_request_id = ?",
            )
            .bind(req_id)
            .fetch_optional(db)
            .await
            .map_err(|e| e.to_string())?
        } else {
            None
        };
    let maybe_existing = if maybe_existing.is_none() {
        sqlx::query_as::<_, crate::models::issue::Issue>(
            "SELECT * FROM issues \
             WHERE project_id = ? AND source_type = 'conversation' AND title = ? \
             AND status = 'pending_execution' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&project_id)
        .bind(&title)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
    } else {
        maybe_existing
    };
    if let Some(ref ex_issue) = maybe_existing {
        if let Some(cr) = sqlx::query_as::<_, crate::models::change_request::ChangeRequest>(
            "SELECT * FROM change_requests WHERE issue_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&ex_issue.id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        {
            return Ok(cr);
        }
        // Issue 存在但 CR 尚无（上次调用中途失败）：补建 CR 后返回。
        return crate::commands::change_requests::create_cr_for_issue(
            db,
            &state.job_tx,
            ex_issue,
            &project,
            Some("会议室「立即编码」：操作者在讨论中确认功能点后直接进入编码"),
            &admin_id,
            work_context.as_deref(),
        )
        .await;
    }
    // ── End 幂等去重 ─────────────────────────────────────────────────────────

    // 创建需求：express 路径直接落 pending_execution（不入分析/需求审核队列）。
    let issue_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO issues (id, project_id, source_type, title, description, category, status, source_ref, client_request_id)
         VALUES (?, ?, 'conversation', ?, ?, 'Feature', 'pending_execution', ?, ?)",
    )
    .bind(&issue_id)
    .bind(&project_id)
    .bind(&title)
    .bind(&brief)
    .bind(&payload.conversation_id)
    .bind(&payload.client_request_id)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    let issue = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&issue_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;

    let cr = crate::commands::change_requests::create_cr_for_issue(
        db,
        &state.job_tx,
        &issue,
        &project,
        Some("会议室「立即编码」：操作者在讨论中确认功能点后直接进入编码"),
        &admin_id,
        work_context.as_deref(),
    )
    .await?;

    // 操作者收件箱 + 总览：与正常录入一致地广播 IssueCreated。
    event::emit(
        &app,
        event::AppEvent::IssueCreated {
            issue_id: issue_id.clone(),
            project_id: project_id.clone(),
        },
    );

    // 回写一条会议室记录，让讨论留痕「这次讨论触发了哪条编码」（best-effort，失败不阻断）。
    let note = format!(
        "⚡ 已据本次讨论创建需求 **{title}** 并立即开始编码（CR `{}`）。进度与代码审核请见「变更审核」页。",
        cr.id
    );
    if let Ok(Some(planner)) = load_planner_agent(db).await {
        if let Ok(message_id) =
            insert_agent_markdown_message(db, &payload.conversation_id, &planner.id, &note).await
        {
            event::emit(
                &app,
                event::AppEvent::MessageReceived {
                    conversation_id: payload.conversation_id.clone(),
                    message_id,
                },
            );
        }
    }

    Ok(cr)
}

/// Recover conversation (会议室) AI tasks orphaned by a previous process exit.
///
/// A task is created `running` and driven by an in-memory `tauri::async_runtime::spawn`
/// task that dies with the process. Unlike pipeline jobs, these are **not** auto-resumed:
/// re-running would re-post AI messages, re-spend LLM tokens, and re-write workspace files
/// — all user-visible side effects. So we close them out as `failed`（注明重启中断）and let
/// the operator re-send the trigger. In-flight steps/runs are closed the same way so the
/// task detail view doesn't show phantom spinners.
///
/// Run ONCE at startup, before any new task can be spawned. DB-only (no Tauri types) so it
/// stays callable from non-Tauri entry points.
pub async fn fail_orphaned_conversation_tasks(db: &crate::db::Db) -> usize {
    const REASON: &str = "任务因程序重启中断，请重新发送指令触发。";
    // Close in-flight steps/runs first so no child row outlives its parent task.
    let _ = sqlx::query(
        "UPDATE conversation_task_runs SET status='failed', completed_at=datetime('now') WHERE status='running'",
    )
    .execute(db)
    .await;
    let _ = sqlx::query(
        "UPDATE conversation_task_steps SET status='failed', error=?, completed_at=datetime('now') WHERE status='running'",
    )
    .bind(REASON)
    .execute(db)
    .await;
    let affected = sqlx::query(
        "UPDATE conversation_tasks SET status='failed', error=?, completed_at=datetime('now') WHERE status='running'",
    )
    .bind(REASON)
    .execute(db)
    .await
    .map(|r| r.rows_affected() as usize)
    .unwrap_or(0);
    if affected > 0 {
        info!(
            "startup recovery: failed {} interrupted conversation task(s)",
            affected
        );
    }
    affected
}

#[tauri::command]
pub async fn list_conversation_tasks(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ConversationTask>, String> {
    sqlx::query_as::<_, ConversationTask>(
        "SELECT * FROM conversation_tasks WHERE conversation_id=? ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompressContextPayload {
    pub conversation_id: String,
    /// "summary"（默认）= 纯压缩摘要；"conclusion" = 收敛结论。
    #[serde(default)]
    pub mode: String,
}

/// 手动触发的「总结内容 / 形成结论」快捷指令：在生成摘要/结论的同时，把当前窗口内的
/// 历史消息全部移出后续上下文（excluded_from_context=1），让摘要本身成为新的上下文基线，
/// 起到压缩上下文的作用。命令体保持薄包装，逻辑下沉到 `compress_context_now`。
#[tauri::command]
pub async fn compress_conversation_context(
    payload: CompressContextPayload,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let trace_tags = crate::core::trace::TraceTags {
        conversation_id: Some(payload.conversation_id.clone()),
        ..Default::default()
    };
    crate::core::trace::with_tags(
        trace_tags,
        compress_context_now(&state.db, &app, &payload.conversation_id, &payload.mode),
    )
    .await
}

async fn compress_context_now(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    mode: &str,
) -> Result<(), String> {
    // 会话串行锁：同一会话的压缩/结论与任务编排互斥，避免并发交织重复插摘要、重复排除原消息。
    // 持锁跨越「读消息→调 LLM→原子写回」全过程，后到的并发请求会在此排队、再读时已是压缩后状态。
    let lock = crate::state::conversation_lock(conversation_id);
    let _guard = lock.lock().await;

    let conversation = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(conversation_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;
    let project_id = conversation.project_id.as_deref();

    let is_conclusion = mode == "conclusion";
    // 结论用 summarizer 角色，纯压缩用 context_compressor；任一缺失则互相回退。
    let primary_kind = if is_conclusion { "summarizer" } else { "context_compressor" };
    let fallback_kind = if is_conclusion { "context_compressor" } else { "summarizer" };
    let (agent, used_kind) = match load_system_role_agent(db, primary_kind).await? {
        Some(a) => (a, primary_kind),
        None => (
            load_system_role_agent(db, fallback_kind)
                .await?
                .ok_or_else(|| "未配置总结/压缩系统角色".to_string())?,
            fallback_kind,
        ),
    };

    let messages = sqlx::query_as::<_, Message>(
        "SELECT *
         FROM messages
         WHERE conversation_id=? AND excluded_from_context=0
         ORDER BY created_at ASC",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    if messages.len() < 2 {
        return Err("当前窗口内的消息太少，无需总结压缩".to_string());
    }
    let source = messages_to_context_text(db, &messages).await?;
    if source.trim().is_empty() {
        return Err("没有可用于总结的消息内容".to_string());
    }

    let prompt = if is_conclusion {
        format!(
            "以下是群聊到目前为止的完整讨论：\n\n{}\n\n请综合各方观点，收敛出明确结论与下一步行动方案。要求：\n- 明确给出结论/裁决及理由。\n- 列出后续行动项（含负责人，如有）。\n- 保留关键约束、分歧与重要事实。\n- 用结构化 Markdown 输出，不要复述完整原文，不要编造未出现的信息。",
            source
        )
    } else {
        format!(
            "以下是群聊到目前为止的完整讨论，请压缩成一份可长期引用的上下文摘要：\n\n{}\n\n要求：\n- 保留需求、决策、约束、待办、分歧和重要事实。\n- 删除寒暄和重复表达。\n- 用结构化 Markdown 输出。\n- 不要编造未出现的信息。",
            source
        )
    };
    let fallback_system = if is_conclusion {
        "你是 AutoForge 的系统总结器，负责把多 Agent 讨论压缩成清晰、可执行、可追溯的结论。"
    } else {
        "你是 AutoForge 的上下文压缩器，负责把长对话压缩成可靠摘要，降低后续 Agent 的上下文负担。"
    };

    let (ok, summary) = run_system_agent_text(
        db,
        &SystemAgentParams {
            conversation_id,
            agent: &agent,
            kind: used_kind,
            prompt: &prompt,
            fallback_system_prompt: fallback_system,
            project_id,
            recall_key: Some(&source),
        },
    )
    .await;
    if !ok {
        return Err(summary);
    }

    let heading = if is_conclusion {
        "讨论结论"
    } else {
        "上下文压缩摘要"
    };
    let markdown = format!(
        "## {}\n\n{}\n\n> 已压缩 {} 条历史消息；原消息仍保留在对话中，但不再进入后续 Agent 上下文。",
        heading,
        summary.trim(),
        messages.len()
    );
    // 原子写回：摘要插入 + 原消息排除同生共死（见 commit_compression）。
    let excluded_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let message_id =
        commit_compression(db, conversation_id, &agent.id, &markdown, &excluded_ids).await?;
    event::emit(
        app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.to_string(),
            message_id,
        },
    );
    Ok(())
}

async fn execute_conversation_task(
    db: crate::db::Db,
    app: AppHandle,
    conversation: Conversation,
    payload: StartConversationTask,
    task_id: String,
) -> Result<(), String> {
    let planner = load_planner_agent(&db).await?;
    let window_size = payload.window_size.unwrap_or(30).clamp(5, 100);
    if let Err(e) = maybe_compress_context(&db, &app, &payload.conversation_id, window_size, conversation.project_id.as_deref()).await {
        warn!("[orchestration] context compression skipped: {}", e);
    }
    let project_prefix =
        crate::commands::project_context::load_project_context_for_conversation(
            &db,
            &payload.conversation_id,
        )
        .await;
    let snapshot = build_context_snapshot(
        &db,
        &payload.conversation_id,
        window_size,
        &project_prefix,
    )
    .await?;
    let members = load_schedulable_members(&db, &payload.conversation_id).await?;
    // Roster of who else is in the room (name + specialty), injected into every
    // agent prompt so agents know whom they can @ to bring in the right expert.
    let roster = build_member_roster(&members);

    let plan = build_plan(
        &db,
        &conversation,
        &payload.instruction,
        &payload.mentioned_agent_ids,
        planner.as_ref(),
        &members,
        &snapshot,
    )
    .await?;
    let plan_json = serde_json::to_string(&plan).map_err(|e| e.to_string())?;

    sqlx::query("UPDATE conversation_tasks SET planner_agent_id=?, plan_json=? WHERE id=?")
        .bind(planner.as_ref().map(|a| a.id.as_str()))
        .bind(&plan_json)
        .bind(&task_id)
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?;

    emit_task_update(&app, &payload.conversation_id, &task_id, "running");

    // Cap on the running transcript of agent replies fed into each subsequent
    // prompt; keeps the most recent tail so growth (steps + chained rounds) is
    // bounded.
    const MAX_ACCUMULATED_BYTES: usize = 12 * 1024;
    let mut accumulated = String::new();
    let mut any_failed = false;
    // Agents that have already spoken in this task (so a chained @mention never
    // re-triggers the same agent and the follow-up phase always terminates).
    let mut triggered: HashSet<String> = HashSet::new();
    // Raw outputs produced in the most recent round, scanned for fresh @mentions.
    let mut pending_texts: Vec<String> = Vec::new();
    let mut next_step_index = 0usize;

    for step in plan.steps.iter() {
        let ctx = ChatTaskCtx {
            db: &db,
            app: &app,
            conversation_id: &payload.conversation_id,
            snapshot: &snapshot,
            accumulated: &accumulated,
            roster: &roster,
            project_id: conversation.project_id.as_deref(),
        };
        let (outcomes, step_failed) = run_plan_step(
            &ctx,
            &task_id,
            next_step_index,
            &step.step_type,
            &step.agents,
            &step.instruction,
        )
        .await?;
        next_step_index += 1;
        any_failed |= step_failed;
        for outcome in &outcomes {
            if !outcome.agent_id.is_empty() {
                triggered.insert(outcome.agent_id.clone());
            }
            accumulated.push_str(&format!(
                "\n\n[{} / {}]\n{}",
                outcome.agent_name, outcome.agent_id, outcome.text
            ));
            pending_texts.push(outcome.text.clone());
        }
        accumulated =
            truncate_keep_tail(&accumulated, MAX_ACCUMULATED_BYTES, "…[较早发言已省略]");
    }

    // Chained @mentions: if an agent's reply @-points at another schedulable
    // member who hasn't spoken yet, let that member respond. Each agent answers
    // at most once via this path, and MAX_CHAIN_ROUNDS hard-caps the depth.
    const MAX_CHAIN_ROUNDS: usize = 4;
    for _ in 0..MAX_CHAIN_ROUNDS {
        let mut to_run: Vec<String> = Vec::new();
        for text in &pending_texts {
            for id in detect_mentioned_agents(text, &members) {
                if !triggered.contains(&id) && !to_run.contains(&id) {
                    to_run.push(id);
                }
            }
        }
        if to_run.is_empty() {
            break;
        }
        let step_type = if to_run.len() > 1 { "parallel" } else { "single" };
        let instruction =
            "你在群聊中被其他 Agent @ 点名。请针对点名你的发言内容作出明确回应（同意/反对/补充并说明理由）。";
        let ctx = ChatTaskCtx {
            db: &db,
            app: &app,
            conversation_id: &payload.conversation_id,
            snapshot: &snapshot,
            accumulated: &accumulated,
            roster: &roster,
            project_id: conversation.project_id.as_deref(),
        };
        let (outcomes, step_failed) = run_plan_step(
            &ctx,
            &task_id,
            next_step_index,
            step_type,
            &to_run,
            instruction,
        )
        .await?;
        next_step_index += 1;
        any_failed |= step_failed;
        pending_texts.clear();
        for outcome in &outcomes {
            if !outcome.agent_id.is_empty() {
                triggered.insert(outcome.agent_id.clone());
            }
            accumulated.push_str(&format!(
                "\n\n[{} / {}]\n{}",
                outcome.agent_name, outcome.agent_id, outcome.text
            ));
            pending_texts.push(outcome.text.clone());
        }
        accumulated =
            truncate_keep_tail(&accumulated, MAX_ACCUMULATED_BYTES, "…[较早发言已省略]");
    }

    // Only fire post-plan system agents when the planner didn't already produce
    // a terminal single step (which would be the business-agent summarizer).
    // This prevents double-summarization when the plan already ends with agent1 summarizing.
    let plan_has_final_single = plan
        .steps
        .last()
        .map(|s| s.step_type == "single")
        .unwrap_or(false);

    if asks_for_synthesis(&payload.instruction) && !plan_has_final_single {
        if let Some(summarizer) = load_system_role_agent(&db, "summarizer").await? {
            let ctx = ChatTaskCtx {
                db: &db,
                app: &app,
                conversation_id: &payload.conversation_id,
                snapshot: &snapshot,
                accumulated: &accumulated,
                roster: &roster,
                project_id: conversation.project_id.as_deref(),
            };
            let outcome = run_summarizer(&ctx, &summarizer, &payload.instruction).await?;
            if !outcome.ok {
                any_failed = true;
            }
            accumulated.push_str(&format!(
                "\n\n[{} / {}]\n{}",
                outcome.agent_name, outcome.agent_id, outcome.text
            ));
        }
    }

    if asks_for_artifact(&payload.instruction) && !plan_has_final_single {
        if let Some(doc_writer) = load_system_role_agent(&db, "doc_writer").await? {
            let ctx = ChatTaskCtx {
                db: &db,
                app: &app,
                conversation_id: &payload.conversation_id,
                snapshot: &snapshot,
                accumulated: &accumulated,
                roster: &roster,
                project_id: conversation.project_id.as_deref(),
            };
            let outcome = run_doc_writer(&ctx, &doc_writer, &payload.instruction).await?;
            if !outcome.ok {
                any_failed = true;
            }
        }
    }

    let final_status = if any_failed { "failed" } else { "completed" };
    sqlx::query(
        "UPDATE conversation_tasks
         SET status=?, completed_at=datetime('now'), error=?
         WHERE id=?",
    )
    .bind(final_status)
    .bind(if any_failed {
        Some("部分步骤执行失败")
    } else {
        None::<&str>
    })
    .bind(&task_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;
    emit_task_update(&app, &payload.conversation_id, &task_id, final_status);
    Ok(())
}

async fn build_plan(
    db: &crate::db::Db,
    conversation: &Conversation,
    instruction: &str,
    mentioned_agent_ids: &[String],
    planner: Option<&Agent>,
    members: &[Agent],
    snapshot: &str,
) -> Result<ConversationPlan, String> {
    if conversation.conv_type == "direct" {
        if let Some(agent) = members.first() {
            return Ok(ConversationPlan {
                steps: vec![ConversationPlanStep {
                    step_type: "single".to_string(),
                    agents: vec![agent.id.clone()],
                    instruction: instruction.to_string(),
                }],
            });
        }
    }

    // 需求录入意图：用户想把内容「加进系统 / 沉淀为需求」，而不是展开讨论。
    // 走专门的需求捕获路径——只派一个 Agent 产出 requirement_draft 草稿块（前端可一键
    // 「提交到流水线」），激活原本休眠的 requirement_draft 链路；单 Agent 也避免并行
    // 多 Agent 各说各话、结论不一致。仅当群聊已绑定项目（草稿才能提交）时启用。
    if conversation.project_id.is_some() && asks_to_capture_issue(instruction) {
        let target = route_by_relevance(instruction, members)
            .or_else(|| members.first().map(|a| a.id.clone()));
        if let Some(agent_id) = target {
            return Ok(ConversationPlan {
                steps: vec![ConversationPlanStep {
                    step_type: "single".to_string(),
                    agents: vec![agent_id],
                    instruction: capture_issue_instruction(instruction),
                }],
            });
        }
    }

    let mentioned: Vec<String> = mentioned_agent_ids
        .iter()
        .filter(|id| members.iter().any(|a| &a.id == *id))
        .cloned()
        .collect();

    // Pure synthesis request with no @mentions: skip Planner and all business-agent steps.
    // Returning an empty plan makes plan_has_final_single=false, so the post-plan
    // summarizer hook fires directly with the conversation snapshot as context.
    //
    // 方向三：收窄触发面。原先只要消息"含"总结/建议/最后等任一词就走纯 summarizer 空计划，
    // 导致"再优化一下并给点建议""那最后这块怎么改"这类日常对话被误判为只综合不回应（伪沉默）。
    // 现要求这是一条很短的、主旨即收口的请求（is_pure_synthesis_request），且确有前一轮 Agent
    // 发言可供综合（last_speaking_member 命中）时才走空计划，否则按普通对话正常选人接话。
    if mentioned.is_empty()
        && is_pure_synthesis_request(instruction)
        && !asks_for_artifact(instruction)
        && last_speaking_member(db, &conversation.id, members)
            .await
            .is_some()
    {
        return Ok(ConversationPlan { steps: vec![] });
    }

    // 方向一 + 方向二：无 @ 且非显式"序列/产物"请求时，用零成本"分诊 + 连续性"直接选出接话人，
    // 跳过 planner 这一跳 LLM——更快、更省，也让每句话都有相关 Agent 接话（无需每次 @）。
    //   1) 相关性分诊：消息字面命中某成员的名字/英文名/角色/专长 → 由该成员接话（等价于"软 @"）。
    //   2) 对话连续性：否则默认由上一轮发言的成员接话，自然承接追问（"再细化一下""那这样呢"）。
    // 两者都没命中（如全新群聊的首条、无关键词消息）才落到 planner / fallback 既有兜底，不放大并发。
    if mentioned.is_empty() && !asks_for_sequence(instruction) && !asks_for_artifact(instruction) {
        let mut target = route_by_relevance(instruction, members);
        if target.is_none() {
            target = last_speaking_member(db, &conversation.id, members).await;
        }
        if let Some(agent_id) = target {
            return Ok(ConversationPlan {
                steps: vec![ConversationPlanStep {
                    step_type: "single".to_string(),
                    agents: vec![agent_id],
                    instruction: instruction.to_string(),
                }],
            });
        }
    }

    let needs_planner = mentioned.is_empty() || asks_for_sequence(instruction);
    if !needs_planner {
        // `@所有人`：被点名的恰好覆盖全部可调度成员（且不止一人）。让全员并发就同一话题
        // 表态，并显式要求互相分析、@ 彼此尽快收敛到一致意见（后续链式 @ 跟进阶段接力）。
        let is_everyone = mentioned.len() > 1 && mentioned.len() == members.len();
        let step_instruction = if is_everyone {
            consensus_instruction(instruction)
        } else {
            instruction.to_string()
        };
        return Ok(ConversationPlan {
            steps: vec![ConversationPlanStep {
                step_type: if mentioned.len() > 1 {
                    "parallel"
                } else {
                    "single"
                }
                .to_string(),
                agents: mentioned,
                instruction: step_instruction,
            }],
        });
    }

    if let Some(planner) = planner {
        match ask_planner(db, planner, instruction, &mentioned, members, snapshot, conversation.project_id.as_deref()).await {
            Ok(plan) => return validate_plan(plan, members, instruction),
            Err(e) => warn!("[orchestration] planner failed, using fallback plan: {}", e),
        }
    }

    fallback_plan(instruction, mentioned, members)
}

async fn ask_planner(
    db: &crate::db::Db,
    planner: &Agent,
    instruction: &str,
    mentioned_agent_ids: &[String],
    members: &[Agent],
    snapshot: &str,
    project_id: Option<&str>,
) -> Result<ConversationPlan, String> {
    let candidates = members
        .iter()
        .map(|a| format!("- id: {}\n  name: {}\n  role: {}", a.id, a.name, a.role))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        r#"将用户请求转换为 AutoForge 群聊编排计划。

候选 Agent：
{}

用户显式 @ 的 Agent ID：
{}

最近对话快照：
{}

用户请求：
{}

只输出 JSON，结构必须是：
{{
  "steps": [
    {{
      "type": "parallel" | "single",
      "agents": ["agent-id"],
      "instruction": "给这些 Agent 的具体任务"
    }}
  ]
}}

约束：
- 只能使用候选 Agent ID。
- parallel 表示同一步内所有 Agent 基于同一快照并发回答。
- single 表示该步骤只有一个 Agent。
- 如果用户要求“然后/最后/总结/裁决/PRD/文档”，用后续 single 步骤表达。
- 不要输出 Markdown，不要解释。"#,
        candidates,
        serde_json::to_string(mentioned_agent_ids).unwrap_or_else(|_| "[]".to_string()),
        snapshot,
        instruction
    );

    // 群聊编排：用注册表内置提示词（按 prompt_mode）+ 以用户请求为键的 Innate 召回。
    let system_prompt = crate::agents::llm::build_role_system_prompt(
        planner,
        Some("planner"),
        None,
        project_id,
        Some(instruction),
    )
    .await;
    // planner 也可按需用工具（如先读真实代码再排计划）；未开启工具时自动回退无工具单轮。
    let tool_ctx = crate::agents::tools::ToolContext::resolve(db, project_id).await;
    let registry = crate::agents::tools::build_registry_for_agent(db, planner, &tool_ctx).await;
    let raw = crate::agents::llm::run_agent_text_with_tools(
        db, planner, &prompt, system_prompt.as_deref(), &[], &registry,
    )
    .await
    .map_err(|e| e.to_string())?;
    parse_plan_json(&raw)
}

fn validate_plan(
    plan: ConversationPlan,
    members: &[Agent],
    fallback_instruction: &str,
) -> Result<ConversationPlan, String> {
    let allowed: HashSet<String> = members.iter().map(|a| a.id.clone()).collect();
    let mut steps = Vec::new();
    for step in plan.steps {
        let mut agents = step
            .agents
            .into_iter()
            .filter(|id| allowed.contains(id))
            .collect::<Vec<_>>();
        agents.dedup();
        if agents.is_empty() {
            continue;
        }
        let step_type = if step.step_type == "parallel" && agents.len() > 1 {
            "parallel".to_string()
        } else {
            agents.truncate(1);
            "single".to_string()
        };
        steps.push(ConversationPlanStep {
            step_type,
            agents,
            instruction: if step.instruction.trim().is_empty() {
                fallback_instruction.to_string()
            } else {
                step.instruction
            },
        });
    }
    if steps.is_empty() {
        return fallback_plan(fallback_instruction, Vec::new(), members);
    }
    Ok(ConversationPlan { steps })
}

fn fallback_plan(
    instruction: &str,
    mentioned_agent_ids: Vec<String>,
    members: &[Agent],
) -> Result<ConversationPlan, String> {
    let agents = if mentioned_agent_ids.is_empty() {
        // 方向一（兜底）：planner 缺失/失败且无 @ 时，优先选与消息最相关的成员，
        // 而非机械地取成员顺序里的第一个；都不相关才退回第一个成员。
        route_by_relevance(instruction, members)
            .or_else(|| members.first().map(|a| a.id.clone()))
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        mentioned_agent_ids
    };
    if agents.is_empty() {
        return Err("当前对话没有可调度的 Agent".to_string());
    }

    // When the user requests a sequence (e.g. "A/B/C discuss then D summarizes") and
    // the planner failed, split the last mentioned agent into a single summary step so
    // the discussion→summary flow is preserved even without LLM planning.
    if asks_for_sequence(instruction) && agents.len() >= 2 {
        let (discussers, summarizer_slice) = agents.split_at(agents.len() - 1);
        let summarizer_id = summarizer_slice[0].clone();
        let discuss_agents = discussers.to_vec();
        return Ok(ConversationPlan {
            steps: vec![
                ConversationPlanStep {
                    step_type: if discuss_agents.len() > 1 {
                        "parallel"
                    } else {
                        "single"
                    }
                    .to_string(),
                    agents: discuss_agents,
                    instruction: instruction.to_string(),
                },
                ConversationPlanStep {
                    step_type: "single".to_string(),
                    agents: vec![summarizer_id],
                    instruction: "请综合以上各方发言，给出总结和裁决建议。".to_string(),
                },
            ],
        });
    }

    Ok(ConversationPlan {
        steps: vec![ConversationPlanStep {
            step_type: if agents.len() > 1 {
                "parallel"
            } else {
                "single"
            }
            .to_string(),
            agents,
            instruction: instruction.to_string(),
        }],
    })
}

/// Run one orchestration step (records the step row, fans the agents out
/// concurrently, then marks the step done). Returns each agent's outcome plus
/// whether any agent in the step failed. Shared by the plan loop and the
/// chained-@mention follow-up phase.
/// Shared, borrowed context threaded through the group-chat (会议室) task
/// execution family — plan steps, per-agent runs, and post-plan system agents
/// (summarizer / doc_writer / markdown) all need the same handful of
/// dependencies plus the running discussion snapshot. Bundling them keeps each
/// helper's signature under the `clippy::too_many_arguments` threshold without
/// changing behavior. All fields are borrows; a `ChatTaskCtx` is cheap to
/// rebuild and only lives for one helper call, so it never outlives the
/// borrowed data.
struct ChatTaskCtx<'a> {
    db: &'a crate::db::Db,
    app: &'a AppHandle,
    conversation_id: &'a str,
    /// Frozen transcript of the conversation fed into each agent prompt.
    snapshot: &'a str,
    /// Running tail of replies produced earlier in this task pass.
    accumulated: &'a str,
    /// Human-readable roster of schedulable members (used by step agents only).
    roster: &'a str,
    project_id: Option<&'a str>,
}

async fn run_plan_step(
    ctx: &ChatTaskCtx<'_>,
    task_id: &str,
    step_index: usize,
    step_type: &str,
    agents: &[String],
    instruction: &str,
) -> Result<(Vec<AgentOutcome>, bool), String> {
    let step_id = Uuid::new_v4().to_string();
    let agent_ids_json = serde_json::to_string(agents).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO conversation_task_steps
         (id, task_id, step_index, step_type, agent_ids_json, instruction, status, started_at)
         VALUES (?, ?, ?, ?, ?, ?, 'running', datetime('now'))",
    )
    .bind(&step_id)
    .bind(task_id)
    .bind(step_index as i64)
    .bind(step_type)
    .bind(&agent_ids_json)
    .bind(instruction)
    .execute(ctx.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut calls = Vec::new();
    for agent_id in agents {
        calls.push(run_agent_for_step(
            ctx,
            task_id,
            &step_id,
            agent_id,
            instruction,
        ));
    }

    let results = join_all(calls).await;
    let mut outcomes = Vec::new();
    let mut step_failed = false;
    for result in results {
        match result {
            Ok(outcome) => {
                if !outcome.ok {
                    step_failed = true;
                }
                outcomes.push(outcome);
            }
            Err(e) => {
                step_failed = true;
                outcomes.push(AgentOutcome {
                    agent_id: String::new(),
                    agent_name: "执行失败".to_string(),
                    ok: false,
                    text: e,
                });
            }
        }
    }

    sqlx::query(
        "UPDATE conversation_task_steps
         SET status=?, completed_at=datetime('now'), error=?
         WHERE id=?",
    )
    .bind(if step_failed { "failed" } else { "completed" })
    .bind(if step_failed {
        Some("部分 Agent 执行失败")
    } else {
        None::<&str>
    })
    .bind(&step_id)
    .execute(ctx.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok((outcomes, step_failed))
}

/// Wrap a user topic with `@所有人` collaboration framing: every member states a
/// clear position, analyzes the others' points, and @-mentions peers to converge
/// on a shared conclusion quickly. The chained-@mention follow-up phase then lets
/// the named members respond, so the discussion actually iterates toward consensus.
fn consensus_instruction(instruction: &str) -> String {
    // 去掉 composer 注入的 `@所有人` 字面前缀，只保留真正的话题。
    let topic = instruction.trim().trim_start_matches("@所有人").trim();
    let topic = if topic.is_empty() { "（见上文对话）" } else { topic };
    format!(
        "群聊全员讨论以下话题，目标是尽快达成一致结论：\n{}\n\n请每位成员：\n\
1. 先基于自身专长给出明确立场和理由；\n\
2. 认真分析其他成员的发言，明确表示同意 / 反对 / 补充，并说明依据；\n\
3. 用 @对方名字 点名你想回应或邀请表态的成员，推动观点收敛；\n\
4. 如果已与他人观点一致，请直接说明并归纳共识，不要为了发言而重复。\n\
保持简洁、聚焦分歧点，避免空泛附和。",
        topic
    )
}

/// Build a human-readable roster of the schedulable group members, one per line
/// as `@名字（角色/专长）`, so each agent knows who else is in the room and whom
/// to @ for a given problem. The `@` prefix matches the mention syntax agents
/// are asked to use.
fn build_member_roster(members: &[Agent]) -> String {
    members
        .iter()
        .filter(|a| !a.name.trim().is_empty())
        .map(|a| {
            let role = a.role.trim();
            if role.is_empty() {
                format!("- @{}", a.name)
            } else {
                format!("- @{}（{}）", a.name, role)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Scan an agent reply for `@AgentName` references to schedulable members.
/// Returns the matched agent ids. A match requires a word boundary after the
/// name so `@设计` does not match member `设计师` and `@Bob` does not match
/// `Bobby`.
fn detect_mentioned_agents(text: &str, members: &[Agent]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for member in members {
        let name = member.name.trim();
        if name.is_empty() {
            continue;
        }
        let needle = format!("@{}", name);
        let mut start = 0usize;
        while let Some(rel) = text[start..].find(&needle) {
            let abs = start + rel;
            let after = abs + needle.len();
            let boundary = match text[after..].chars().next() {
                None => true,
                Some(c) => !(c.is_alphanumeric() || c == '_'),
            };
            if boundary {
                if !found.contains(&member.id) {
                    found.push(member.id.clone());
                }
                break;
            }
            start = after;
            if start >= text.len() {
                break;
            }
        }
    }
    found
}

async fn run_agent_for_step(
    ctx: &ChatTaskCtx<'_>,
    task_id: &str,
    step_id: &str,
    agent_id: &str,
    instruction: &str,
) -> Result<AgentOutcome, String> {
    let run_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO conversation_task_runs
         (id, task_id, step_id, agent_id, status, started_at)
         VALUES (?, ?, ?, ?, 'running', datetime('now'))",
    )
    .bind(&run_id)
    .bind(task_id)
    .bind(step_id)
    .bind(agent_id)
    .execute(ctx.db)
    .await
    .map_err(|e| e.to_string())?;

    let agent = load_agent(ctx.db, agent_id).await?;
    // 编码 Agent 后端成员：改走 CLI 只读跑项目仓库作答的路径（不经 LLM tool-loop）。
    // 复用已插入的 run_id 这条 conversation_task_runs 行。
    if let Some(code_agent_id) = agent.code_agent_id.as_deref().filter(|s| !s.trim().is_empty()) {
        return run_code_agent_reply(ctx, &run_id, &agent, code_agent_id, instruction).await;
    }
    let roster_section = if ctx.roster.trim().is_empty() {
        String::new()
    } else {
        format!(
            "群聊成员名单（了解在场成员；话题相关时可 @名字 点名协作）：\n{}\n\n",
            ctx.roster
        )
    };
    let prompt = format!(
        "{}以下是群聊对话快照：\n{}\n\n前置 Agent 发言：\n{}\n\n当前任务：\n{}\n\n请以 {} 的身份在群聊中直接回复。保持观点明确，必要时输出结构化 Markdown。\n优先自己把问题答完，不必为了协作而刻意 @ 别人；但当某部分确实更适合其他成员的专长、或你想就分歧点邀请其表态时，可以自然地用 @对方名字 点名（仅 @ 名单中的成员，且只 @ 与当前话题真正相关的成员）。不要为了凑发言或客套而 @ 无关成员。",
        roster_section,
        ctx.snapshot,
        if ctx.accumulated.trim().is_empty() { "无" } else { ctx.accumulated },
        instruction,
        agent.name
    );
    // 统一输出规范：自由对话 Agent 的发言需条理清晰、简洁准确。把规范追加到 Agent
    // 自身系统提示词之后（无自定义提示词时单独使用），不影响有严格输出契约的系统角色。
    let base_prompt = agent.system_prompt.trim();
    let system_prompt = if base_prompt.is_empty() {
        crate::agents::roles::OUTPUT_FORMAT_GUIDE.to_string()
    } else {
        format!(
            "{}\n\n{}",
            base_prompt,
            crate::agents::roles::OUTPUT_FORMAT_GUIDE
        )
    };
    // 群聊步骤 Agent 的工具集：内置工具(capabilities 白名单) + 只读代码情报(绑定项目时无条件补齐)
    // + 勾选的 MCP server 工具。用 chat 版装配，确保群聊里 Agent 总能真正读到项目代码而非空口承诺。
    let tool_ctx = crate::agents::tools::ToolContext::resolve(ctx.db, ctx.project_id).await;
    let registry =
        crate::agents::tools::build_registry_for_chat_agent(ctx.db, &agent, &tool_ctx).await;
    // 多模态：收集最近上下文窗口内的图片附件，交给绑定多模态 LLM 的 Agent 识别。
    // 非多模态 LLM 会在 llm 层静默忽略这些图片（快照里仍保留「[图片: …]」文字描述）。
    let images = collect_context_images(ctx.db, ctx.conversation_id, 40, 6).await;
    // 实时活动流：把 Agent 的工具动作与回复正文逐字增量转成 `AgentThinking` 事件推给前端，
    // 消除会议室「干等」的等待感。闭包只捕获 AppHandle + 标识串，业务层（llm.rs）对 Tauri 无感知。
    // seq 在本次执行内递增，前端据此按序拼接；run_id 区分并行步骤里同时发言的多个 Agent。
    let think_app = ctx.app;
    let think_conv = ctx.conversation_id.to_string();
    let think_run = run_id.clone();
    let think_aid = agent_id.to_string();
    let think_aname = agent.name.clone();
    let mut think_seq: u64 = 0;
    let mut on_think = move |ev: crate::agents::llm::ThinkEvent| {
        let (kind, text) = match ev {
            crate::agents::llm::ThinkEvent::Token(t) => ("token", t),
            crate::agents::llm::ThinkEvent::Tool { summary, .. } => ("tool", summary),
        };
        event::emit(
            think_app,
            event::AppEvent::AgentThinking {
                conversation_id: think_conv.clone(),
                run_id: think_run.clone(),
                agent_id: think_aid.clone(),
                agent_name: think_aname.clone(),
                kind: kind.to_string(),
                text,
                seq: think_seq,
            },
        );
        think_seq += 1;
    };
    // 把「LLM 调用 + 解析 <write-file> + 落盘」包进同一个 trace run：写文件以 tool span
    // 挂在与本次 Agent 调用相同的 trace 下，链路追踪里即可审计 Agent 写了哪些工作区文件。
    let (ok, text, text_after_writes, error, write_blocks) =
        crate::core::trace::scope_run(ctx.db, &agent, async {
            let result = crate::agents::llm::run_agent_text_with_tools_streaming(
                ctx.db, &agent, &prompt, Some(system_prompt.as_str()), &images, &registry,
                &mut on_think,
            )
            .await;
            let (ok, text, error) = match result {
                Ok(text) => (true, text, None),
                Err(e) => {
                    let msg = format!("[系统错误: {}]", e);
                    (false, msg, Some(e.to_string()))
                }
            };
            // Parse file writes first (before requirement draft extraction)
            let (text_after_writes, file_writes) =
                crate::commands::workspace::parse_agent_file_writes(&text);
            let write_blocks =
                crate::commands::workspace::execute_agent_writes(ctx.db, ctx.conversation_id, file_writes)
                    .await;
            (ok, text, text_after_writes, error, write_blocks)
        })
        .await;

    // 检测 LLM 输出中是否嵌入了 requirement_draft artifact JSON
    let (clean_text, draft_artifact) = extract_issue_draft_artifact(&text_after_writes);
    let mut blocks = vec![serde_json::json!({ "t": "md", "md": clean_text })];
    if let Some(mut artifact) = draft_artifact {
        // LLM 只产出 requirement_draft 的业务字段，这里补齐前端渲染/提交所需：
        // 1) 打上 block 类型 t=artifact，否则 Block.tsx 不会渲染成 artifact 块；
        // 2) 用已知会话项目 id 覆盖 _meta.project_id（不信任 LLM 自填），
        //    使「提交到流水线」按钮真正可用。
        if let Some(obj) = artifact.as_object_mut() {
            obj.insert("t".to_string(), serde_json::json!("artifact"));
            if let Some(pid) = ctx.project_id {
                let meta = obj
                    .entry("_meta")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(meta_obj) = meta.as_object_mut() {
                    meta_obj.insert("project_id".to_string(), serde_json::json!(pid));
                }
            }
        }
        blocks.push(artifact);
    }
    for wb in write_blocks {
        blocks.push(wb);
    }
    finalize_chat_reply(ctx, &run_id, agent_id, &agent.name, &blocks, ok, text, error).await
}

/// 编码 Agent 后端成员的群聊回复：在项目仓库内**只读**跑 CLI（claude/codex）回答问题，把实时
/// 活动转成「思考」流推前端，最终落库末轮答案。需群聊**绑定带仓库的项目**（要有真实代码可读）；
/// 未绑定 / 解析失败 / 该 kind 不支持只读问答（opencode）时，落一条说明性消息优雅降级，
/// 不抛错卡住整个任务。复用调用方已插入的 `run_id` 运行行。
async fn run_code_agent_reply(
    ctx: &ChatTaskCtx<'_>,
    run_id: &str,
    agent: &Agent,
    code_agent_id: &str,
    instruction: &str,
) -> Result<AgentOutcome, String> {
    // 1) 需要项目仓库（只读读取真实代码）。
    let repo_path = match ctx.project_id {
        Some(pid) => sqlx::query_as::<_, crate::models::project::Project>(
            "SELECT * FROM projects WHERE id=?",
        )
        .bind(pid)
        .fetch_optional(ctx.db)
        .await
        .map_err(|e| e.to_string())?
        .map(|p| p.repo_path)
        .unwrap_or_default(),
        None => String::new(),
    };
    if repo_path.trim().is_empty() {
        let msg = format!(
            "我是编码 Agent 成员「{}」，需要本群聊**绑定一个带仓库路径的项目**才能读取真实代码作答。\
             请在群聊设置里绑定项目后再 @ 我。",
            agent.name
        );
        let blocks = vec![serde_json::json!({ "t": "md", "md": msg.clone() })];
        return finalize_chat_reply(ctx, run_id, &agent.id, &agent.name, &blocks, true, msg, None)
            .await;
    }

    // 2) 解析该成员绑定的编码 Agent（查不到/停用即降级，不静默兜底 claude）。
    let Some(code_agent) =
        crate::agents::code_agent::resolve_by_id(ctx.db, code_agent_id).await
    else {
        let msg = format!(
            "我绑定的编码 Agent 不可用（可能已被删除或停用）。请在「设置 → Agent」里为成员「{}」\
             重新指定编码 Agent 后端。",
            agent.name
        );
        let blocks = vec![serde_json::json!({ "t": "md", "md": msg.clone() })];
        return finalize_chat_reply(ctx, run_id, &agent.id, &agent.name, &blocks, true, msg, None)
            .await;
    };

    // 3) prompt：群聊上下文 + 只读读真实代码的措辞（不输出写文件指令）。
    let roster_section = if ctx.roster.trim().is_empty() {
        String::new()
    } else {
        format!(
            "群聊成员名单（话题相关时可 @名字 协作）：\n{}\n\n",
            ctx.roster
        )
    };
    let prompt = format!(
        "{roster}以下是群聊对话快照：\n{snap}\n\n前置 Agent 发言：\n{acc}\n\n当前任务：\n{ins}\n\n\
         你是群聊成员「{name}」，可直接读取并检索本项目仓库的**真实代码**来回答。请基于真实代码\
         作答（点明涉及的文件/符号与关键流程），结论先行、观点明确，用简洁中文 Markdown。\
         你处于**只读**模式：可读代码、检索、分析，但不会改动任何文件，也不要输出文件写入指令。\
         若某部分确实更适合其他成员，可自然地用 @对方名字 点名（仅 @ 名单中相关成员），\
         不要为凑协作而 @ 无关成员。",
        roster = roster_section,
        snap = ctx.snapshot,
        acc = if ctx.accumulated.trim().is_empty() { "无" } else { ctx.accumulated },
        ins = instruction,
        name = agent.name,
    );

    // 4) 限额复用「执行」配置；MCP 复用「适用于编码 Agent」的只读情报（如 codegraph）。
    let (wall_secs, idle_secs) = crate::commands::system::load_execution_limits(ctx.db).await;
    let limits = crate::agents::code_agent::RunLimits { wall_secs, idle_secs };
    let code_mcp = crate::agents::code_agent::load_code_agent_mcp(ctx.db).await;

    // 5) 实时活动 → 「思考」流：把 CLI 边跑边吐的可读增量转成 AgentThinking(token) 推前端，
    //    消除会议室干等。业务层对 Tauri 仅经 event::emit 感知。
    let (log_tx, mut log_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::agents::code_agent::LogChunk>();
    let forward = {
        let app = ctx.app.clone();
        let conv = ctx.conversation_id.to_string();
        let rid = run_id.to_string();
        let aid = agent.id.clone();
        let aname = agent.name.clone();
        tokio::spawn(async move {
            let mut seq: u64 = 0;
            while let Some(c) = log_rx.recv().await {
                event::emit(
                    &app,
                    event::AppEvent::AgentThinking {
                        conversation_id: conv.clone(),
                        run_id: rid.clone(),
                        agent_id: aid.clone(),
                        agent_name: aname.clone(),
                        kind: "token".to_string(),
                        text: c.text,
                        seq,
                    },
                );
                seq += 1;
            }
        })
    };

    // 6) 只读跑仓库取末轮答案。
    let result = code_agent
        .answer(&repo_path, &prompt, limits, &code_mcp, Some(&log_tx))
        .await;
    drop(log_tx); // 关闭 sink，让转发任务收尾
    let _ = forward.await;

    let (ok, text, error) = match result {
        Ok(t) => (true, t, None),
        Err(e) => (
            false,
            format!("[编码 Agent「{}」执行失败] {}", agent.name, e),
            Some(e.to_string()),
        ),
    };
    let blocks = vec![serde_json::json!({ "t": "md", "md": text.clone() })];
    finalize_chat_reply(ctx, run_id, &agent.id, &agent.name, &blocks, ok, text, error).await
}

/// 落库一条 Agent 群聊回复（LLM 后端与编码 Agent 后端两路共用）：写 `messages`、更新本次
/// `conversation_task_runs`（状态/output_text/error）、广播 `MessageReceived` 并撤下实时活动
/// 卡片，返回 `AgentOutcome`（其 `text` 供后续轮次 / 链式 @ 累计）。`blocks` 为已构建好的消息
/// 块数组（md / artifact / file_written…），`run_id` 为调用方已插入的运行行 id。
#[allow(clippy::too_many_arguments)]
async fn finalize_chat_reply(
    ctx: &ChatTaskCtx<'_>,
    run_id: &str,
    agent_id: &str,
    agent_name: &str,
    blocks: &[serde_json::Value],
    ok: bool,
    text: String,
    error: Option<String>,
) -> Result<AgentOutcome, String> {
    let content_json = serde_json::to_string(blocks).unwrap_or_default();

    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(ctx.conversation_id)
    .bind(agent_id)
    .bind(&content_json)
    .execute(ctx.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE conversation_task_runs
         SET status=?, completed_at=datetime('now'), message_id=?, output_text=?, error=?
         WHERE id=?",
    )
    .bind(if ok { "completed" } else { "failed" })
    .bind(&message_id)
    .bind(&text)
    .bind(&error)
    .bind(run_id)
    .execute(ctx.db)
    .await
    .map_err(|e| e.to_string())?;

    event::emit(
        ctx.app,
        event::AppEvent::MessageReceived {
            conversation_id: ctx.conversation_id.to_string(),
            message_id,
        },
    );
    // 收尾：通知前端撤下本次执行的实时活动卡片，换成刚落库的正式消息气泡。
    event::emit(
        ctx.app,
        event::AppEvent::AgentThinking {
            conversation_id: ctx.conversation_id.to_string(),
            run_id: run_id.to_string(),
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            kind: "done".to_string(),
            text: String::new(),
            seq: u64::MAX,
        },
    );

    Ok(AgentOutcome {
        agent_id: agent_id.to_string(),
        agent_name: agent_name.to_string(),
        ok,
        text,
    })
}

async fn run_summarizer(
    ctx: &ChatTaskCtx<'_>,
    agent: &Agent,
    instruction: &str,
) -> Result<AgentOutcome, String> {
    let prompt = format!(
        "以下是群聊对话快照：\n{}\n\n本轮 Agent 发言：\n{}\n\n用户原始请求：\n{}\n\n请作为群聊总结器输出最终结论。要求：\n- 综合各方观点，不重复完整原文。\n- 如果用户要求裁决，明确给出裁决和理由。\n- 输出后续行动建议。\n- 使用结构化 Markdown。",
        ctx.snapshot,
        if ctx.accumulated.trim().is_empty() { "无" } else { ctx.accumulated },
        instruction
    );
    let fallback_system =
        "你是 AutoForge 的系统总结器，负责把多 Agent 讨论压缩成清晰、可执行、可追溯的结论。";
    run_system_agent_markdown(
        ctx,
        &SystemAgentParams {
            conversation_id: ctx.conversation_id,
            agent,
            kind: "summarizer",
            prompt: &prompt,
            fallback_system_prompt: fallback_system,
            project_id: ctx.project_id,
            recall_key: Some(instruction),
        },
    )
    .await
}

async fn run_doc_writer(
    ctx: &ChatTaskCtx<'_>,
    agent: &Agent,
    instruction: &str,
) -> Result<AgentOutcome, String> {
    let default_kind = infer_artifact_kind(instruction);
    let prompt = format!(
        r#"以下是群聊对话快照：
{}

本轮讨论和总结：
{}

用户原始请求：
{}

请生成一个可沉淀的文档产物。只输出 JSON，不要 Markdown，不要解释。
JSON 结构：
{{
  "kind": "{}",
  "title": "文档标题",
  "rows": [["状态", "草案"], ["来源", "群聊讨论"]],
  "body": "完整正文，使用 Markdown 风格的小标题和列表，但必须作为 JSON 字符串"
}}

要求：
- body 要可直接作为 PRD、ADR、测试计划或实施方案的初稿使用。
- 不要遗漏背景、目标、范围、约束、风险和下一步。
- rows 控制在 3 到 6 行。"#,
        ctx.snapshot,
        if ctx.accumulated.trim().is_empty() {
            "无"
        } else {
            ctx.accumulated
        },
        instruction,
        default_kind
    );
    let fallback_system =
        "你是 AutoForge 的系统文档生成器，负责把群聊讨论沉淀为可引用、可迭代的文档产物。";
    let (ok, raw) = run_system_agent_text(
        ctx.db,
        &SystemAgentParams {
            conversation_id: ctx.conversation_id,
            agent,
            kind: "doc_writer",
            prompt: &prompt,
            fallback_system_prompt: fallback_system,
            project_id: ctx.project_id,
            recall_key: Some(instruction),
        },
    )
    .await;
    if !ok {
        let message_id =
            insert_agent_markdown_message(ctx.db, ctx.conversation_id, &agent.id, &raw).await?;
        event::emit(
            ctx.app,
            event::AppEvent::MessageReceived {
                conversation_id: ctx.conversation_id.to_string(),
                message_id,
            },
        );
        return Ok(AgentOutcome {
            agent_id: agent.id.clone(),
            agent_name: agent.name.clone(),
            ok,
            text: raw,
        });
    }

    let artifact = parse_artifact_payload(&raw, instruction);
    let text_for_trace = format!(
        "[{}] {}\n\n{}",
        artifact.kind, artifact.title, artifact.body
    );
    insert_agent_artifact_message(ctx.db, ctx.app, ctx.conversation_id, &agent.id, artifact).await?;

    Ok(AgentOutcome {
        agent_id: agent.id.clone(),
        agent_name: agent.name.clone(),
        ok,
        text: if ok { text_for_trace } else { raw },
    })
}

async fn run_system_agent_markdown(
    ctx: &ChatTaskCtx<'_>,
    params: &SystemAgentParams<'_>,
) -> Result<AgentOutcome, String> {
    let (ok, text) = run_system_agent_text(ctx.db, params).await;
    let message_id =
        insert_agent_markdown_message(ctx.db, params.conversation_id, &params.agent.id, &text)
            .await?;
    event::emit(
        ctx.app,
        event::AppEvent::MessageReceived {
            conversation_id: params.conversation_id.to_string(),
            message_id,
        },
    );
    Ok(AgentOutcome {
        agent_id: params.agent.id.clone(),
        agent_name: params.agent.name.clone(),
        ok,
        text,
    })
}

async fn run_system_agent_text(
    db: &crate::db::Db,
    params: &SystemAgentParams<'_>,
) -> (bool, String) {
    // 用注册表内置提示词（按 prompt_mode）+ Innate 召回，让群聊系统角色随经验越用越好。
    let system_prompt = crate::agents::llm::build_role_system_prompt(
        params.agent,
        Some(params.kind),
        Some(params.fallback_system_prompt),
        params.project_id,
        params.recall_key,
    )
    .await;
    // 系统角色也可按需用工具（代码扫描/web_search）：注册表按 capabilities 白名单 + 项目绑定装配；
    // 为空（未开启工具/无项目）时 run_agent_text_with_tools 自动回退到无工具单轮，行为不变。
    let tool_ctx = crate::agents::tools::ToolContext::resolve(db, params.project_id).await;
    let registry =
        crate::agents::tools::build_registry_for_agent(db, params.agent, &tool_ctx).await;
    match crate::agents::llm::run_agent_text_with_tools(
        db,
        params.agent,
        params.prompt,
        system_prompt.as_deref(),
        &[],
        &registry,
    )
    .await
    {
        Ok(text) => (true, text),
        Err(e) => (false, format!("[系统错误: {}]", e)),
    }
}

async fn maybe_compress_context(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    window_size: i64,
    project_id: Option<&str>,
) -> Result<(), String> {
    let Some(compressor) = load_system_role_agent(db, "context_compressor").await? else {
        return Ok(());
    };
    let messages = sqlx::query_as::<_, Message>(
        "SELECT *
         FROM messages
         WHERE conversation_id=? AND excluded_from_context=0
         ORDER BY created_at ASC",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    let threshold = window_size as usize;
    if messages.len() <= threshold {
        return Ok(());
    }
    let keep_recent = (threshold / 2).max(5).min(threshold.saturating_sub(1));
    let compress_count = messages.len().saturating_sub(keep_recent);
    if compress_count < 2 {
        return Ok(());
    }

    let to_compress = &messages[..compress_count];
    let source = messages_to_context_text(db, to_compress).await?;
    if source.trim().is_empty() {
        return Ok(());
    }

    let prompt = format!(
        "下面是将从后续上下文窗口中排除的较早群聊消息，请压缩成一份可长期引用的上下文摘要。\n\n消息：\n{}\n\n要求：\n- 保留需求、决策、约束、待办、分歧和重要事实。\n- 删除寒暄和重复表达。\n- 用结构化 Markdown 输出。\n- 不要编造未出现的信息。",
        source
    );
    let fallback_system = "你是 AutoForge 的上下文压缩器，负责在长对话超过窗口条数后生成可靠摘要，降低后续 Agent 的上下文负担。";
    // 召回键用待压缩内容，命中该对话主题相关的项目经验。
    let (ok, summary) = run_system_agent_text(
        db,
        &SystemAgentParams {
            conversation_id,
            agent: &compressor,
            kind: "context_compressor",
            prompt: &prompt,
            fallback_system_prompt: fallback_system,
            project_id,
            recall_key: Some(&source),
        },
    )
    .await;
    if !ok {
        return Err(summary);
    }

    let markdown = format!(
        "## 上下文压缩摘要\n\n{}\n\n> 已压缩 {} 条较早消息；原消息仍保留在对话中，但不再进入后续 Agent 上下文。",
        summary.trim(),
        to_compress.len()
    );
    // 原子写回：摘要插入 + 原消息排除同生共死（见 commit_compression）。
    let excluded_ids: Vec<String> = to_compress.iter().map(|m| m.id.clone()).collect();
    let message_id =
        commit_compression(db, conversation_id, &compressor.id, &markdown, &excluded_ids).await?;
    event::emit(
        app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.to_string(),
            message_id,
        },
    );
    Ok(())
}

async fn build_context_snapshot(
    db: &crate::db::Db,
    conversation_id: &str,
    limit: i64,
    project_prefix: &str,
) -> Result<String, String> {
    let candidates = sqlx::query_as::<_, Message>(
        "SELECT *
         FROM messages
         WHERE conversation_id=?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    let ordered = candidates.iter().rev().cloned().collect::<Vec<_>>();
    let conv_text = messages_to_context_text(db, &ordered).await?;
    // The message window is bounded by count, not size; a few large messages
    // (e.g. pasted files) can still blow it up. Cap by bytes, keeping the most
    // recent tail since that is what agents reason about.
    const MAX_SNAPSHOT_BYTES: usize = 24 * 1024;
    let conv_text = truncate_keep_tail(&conv_text, MAX_SNAPSHOT_BYTES, "…[较早对话已省略]");

    if project_prefix.is_empty() {
        return Ok(conv_text);
    }
    Ok(format!("{}{}", project_prefix, conv_text))
}

/// 组装「会议室立即编码」用的上下文：项目上下文（claude.md/agents.md/pinned 文件/工作区
/// 文件清单）+ 最近 `window` 条对话快照。复用与 AI 任务完全相同的取材，保证交给编码 agent
/// 的背景与会议室里看到的一致。
pub(crate) async fn assemble_conversation_context(
    db: &crate::db::Db,
    conversation_id: &str,
    window: i64,
) -> Result<String, String> {
    let project_prefix =
        crate::commands::project_context::load_project_context_for_conversation(db, conversation_id)
            .await;
    build_context_snapshot(db, conversation_id, window, &project_prefix).await
}

/// 加载用于「梳理功能点」的 Agent：优先 forge_role=analysis 的分析 Agent（绑定低成本 LLM），
/// 否则回退 planner 系统角色。两者皆缺 → 报错，提示去设置绑定 LLM。
async fn load_brief_agent(db: &crate::db::Db) -> Result<Agent, String> {
    if let Some(a) = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(forge_role, '') || ',') LIKE '%,analysis,%'
           AND llm_id IS NOT NULL
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    {
        return Ok(a);
    }
    if let Some(a) = load_planner_agent(db).await? {
        return Ok(a);
    }
    Err("未配置可用于梳理需求的 Agent（请在设置中为「分析」或 planner 角色绑定 LLM）".to_string())
}

/// Truncate a string to at most `max_bytes`, keeping the tail (most recent
/// content) on a char boundary and prepending `notice` when content was dropped.
fn truncate_keep_tail(s: &str, max_bytes: usize, notice: &str) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    format!("{}\n\n{}", notice, &s[start..])
}

async fn messages_to_context_text(
    db: &crate::db::Db,
    messages: &[Message],
) -> Result<String, String> {
    let agent_rows = sqlx::query_as::<_, Agent>("SELECT * FROM agents")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    let agent_names: HashMap<String, String> =
        agent_rows.into_iter().map(|a| (a.id, a.name)).collect();

    let mut parts = Vec::new();
    for msg in messages.iter().filter(|m| !m.excluded_from_context) {
        let sender = msg
            .from_agent
            .as_ref()
            .and_then(|id| agent_names.get(id))
            .cloned()
            .unwrap_or_else(|| "User".to_string());
        let text = message_to_prompt_text(db, msg).await?;
        if !text.trim().is_empty() {
            parts.push(format!("[{}]\n{}", sender, text));
        }
    }
    Ok(parts.join("\n\n"))
}

/// 原子提交一次压缩/结论：在**单事务**内插入摘要消息 + 把被压缩的原消息标记为
/// `excluded_from_context=1`。要么全成功要么全回滚——杜绝「摘要已插入但原消息未排除
/// （下轮重复压缩、摘要叠摘要）」或「原消息已排除但摘要插入失败（内容凭空丢失）」的半完成态。
async fn commit_compression(
    db: &crate::db::Db,
    conversation_id: &str,
    agent_id: &str,
    markdown: &str,
    excluded_ids: &[String],
) -> Result<String, String> {
    let content_json = serde_json::json!([{ "t": "md", "md": markdown }]).to_string();
    let message_id = Uuid::new_v4().to_string();
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json) VALUES (?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(agent_id)
    .bind(&content_json)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    for id in excluded_ids {
        sqlx::query("UPDATE messages SET excluded_from_context=1 WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(message_id)
}

async fn insert_agent_markdown_message(
    db: &crate::db::Db,
    conversation_id: &str,
    agent_id: &str,
    text: &str,
) -> Result<String, String> {
    let content_json = serde_json::json!([{ "t": "md", "md": text }]).to_string();
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(agent_id)
    .bind(&content_json)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(message_id)
}

async fn insert_agent_artifact_message(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    agent_id: &str,
    artifact: ArtifactPayload,
) -> Result<String, String> {
    let content_json = serde_json::json!([{
        "t": "artifact",
        "kind": artifact.kind,
        "title": artifact.title,
        "rows": artifact.rows,
        "body": artifact.body,
    }])
    .to_string();
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(conversation_id)
    .bind(agent_id)
    .bind(&content_json)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    event::emit(
        app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.to_string(),
            message_id: message_id.clone(),
        },
    );
    Ok(message_id)
}

async fn message_to_prompt_text(db: &crate::db::Db, msg: &Message) -> Result<String, String> {
    let blocks: Vec<serde_json::Value> = serde_json::from_str(&msg.content_json)
        .unwrap_or_else(|_| vec![serde_json::json!({ "t": "md", "md": msg.content_json })]);
    let mut parts = Vec::new();
    for block in &blocks {
        match block.get("t").and_then(|v| v.as_str()) {
            Some("md") => {
                if let Some(md) = block.get("md").and_then(|v| v.as_str()) {
                    parts.push(md.to_string());
                }
            }
            Some("code") => {
                let lang = block.get("lang").and_then(|v| v.as_str()).unwrap_or("");
                let code = block.get("code").and_then(|v| v.as_str()).unwrap_or("");
                parts.push(format!("```{}\n{}\n```", lang, code));
            }
            Some("artifact") => {
                let kind = block
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("artifact");
                let title = block.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body = block.get("body").and_then(|v| v.as_str()).unwrap_or("");
                parts.push(format!("[{}: {}]\n{}", kind, title, body));
            }
            Some("quote_ref") => {
                let author = block.get("author").and_then(|v| v.as_str()).unwrap_or("?");
                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                parts.push(format!("[引用 {}]: {}", author, text));
            }
            Some("file") => {
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("文件");
                if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                    match load_text_attachment_content(db, id).await {
                        Ok(Some(content)) => {
                            parts.push(format!("[文件内容 - {}]\n```\n{}\n```", name, content))
                        }
                        Ok(None) => parts.push(format!("[附件: {}]", name)),
                        Err(e) => parts.push(format!("[附件: {} 读取失败: {}]", name, e)),
                    }
                }
            }
            Some("image") => {
                let label = block
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("图片");
                parts.push(format!("[图片: {}]", label));
            }
            Some("ws_ref") => {
                let path = block.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if !path.is_empty() {
                    match load_workspace_ref_content(db, &msg.conversation_id, path).await {
                        Ok(content) => parts.push(format!(
                            "[工作区文件引用 - .autoforge/{}]\n```\n{}\n```",
                            path, content
                        )),
                        Err(e) => parts.push(format!(
                            "[工作区文件引用: .autoforge/{} 读取失败: {}]",
                            path, e
                        )),
                    }
                }
            }
            _ => {}
        }
    }
    Ok(parts.join("\n"))
}

async fn load_text_attachment_content(
    db: &crate::db::Db,
    attachment_id: &str,
) -> Result<Option<String>, String> {
    let attachment = sqlx::query_as::<_, ConversationAttachment>(
        "SELECT * FROM conversation_attachments WHERE id=?",
    )
    .bind(attachment_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "附件不存在".to_string())?;

    const TEXT_MIMES: &[&str] = &[
        "text/plain",
        "text/markdown",
        "text/csv",
        "application/json",
        "application/x-yaml",
        "application/toml",
    ];
    if !TEXT_MIMES.contains(&attachment.mime.as_str()) {
        return Ok(None);
    }
    let path = attachment_path(&attachment)?;
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "文件不是有效的 UTF-8 文本".to_string())?;
    const MAX_CHARS: usize = 50_000;
    if text.chars().count() > MAX_CHARS {
        Ok(Some(text.chars().take(MAX_CHARS).collect::<String>()))
    } else {
        Ok(Some(text.to_string()))
    }
}

/// 解析消息携带的工作区文件引用（ws_ref 块）：经会话定位项目 repo_path，
/// 再用 workspace 守卫读取 .autoforge/ 下的文件内容（限 docs/specs，禁越界），
/// 构建 Agent 提示时按需调用，避免把全文塞进存储的消息。
async fn load_workspace_ref_content(
    db: &crate::db::Db,
    conversation_id: &str,
    rel_under_autoforge: &str,
) -> Result<String, String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT p.repo_path FROM conversations c \
         JOIN projects p ON p.id = c.project_id WHERE c.id = ?",
    )
    .bind(conversation_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    let (repo_path,) = row.ok_or_else(|| "会话未绑定项目".to_string())?;
    if repo_path.is_empty() {
        return Err("项目未配置仓库路径".to_string());
    }
    let content =
        crate::commands::workspace::read_workspace_path(&repo_path, rel_under_autoforge).await?;
    // 防止单个引用文件撑爆上下文，与文本附件一致截断到 50k 字符。
    const MAX_CHARS: usize = 50_000;
    if content.chars().count() > MAX_CHARS {
        Ok(content.chars().take(MAX_CHARS).collect::<String>())
    } else {
        Ok(content)
    }
}

fn attachment_path(attachment: &ConversationAttachment) -> Result<PathBuf, String> {
    let rel = Path::new(&attachment.rel_path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("附件路径无效".to_string());
    }
    Ok(PathBuf::from(crate::state::attachments_base()).join(rel))
}

/// 收集会议室最近上下文窗口内的图片附件路径（按时间顺序，最多 `max_images` 张），
/// 供多模态 Agent 识别。扫描最近 `limit` 条未移出上下文的消息中的 `image` 块，解析其
/// 附件 id 为磁盘路径；非图片 / 路径非法的项忽略。best-effort，任何查询失败返回空。
async fn collect_context_images(
    db: &crate::db::Db,
    conversation_id: &str,
    limit: i64,
    max_images: usize,
) -> Vec<PathBuf> {
    let messages = match sqlx::query_as::<_, Message>(
        "SELECT * FROM messages
         WHERE conversation_id=? AND excluded_from_context=0
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(db)
    .await
    {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    // messages 为时间倒序（最新在前）；从最新往回收集图片附件 id，凑满上限即停。
    let mut ids: Vec<String> = Vec::new();
    for msg in &messages {
        let blocks: Vec<serde_json::Value> =
            serde_json::from_str(&msg.content_json).unwrap_or_default();
        for block in &blocks {
            if block.get("t").and_then(|v| v.as_str()) == Some("image") {
                if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                    ids.push(id.to_string());
                }
            }
        }
        if ids.len() >= max_images {
            break;
        }
    }
    ids.truncate(max_images);
    // 还原为时间正序（最旧在前），让多模态请求里的图片顺序贴近对话阅读顺序。
    ids.reverse();

    let mut paths = Vec::new();
    for id in ids {
        if let Ok(Some(att)) = sqlx::query_as::<_, ConversationAttachment>(
            "SELECT * FROM conversation_attachments WHERE id=?",
        )
        .bind(&id)
        .fetch_optional(db)
        .await
        {
            if att.kind == "image" {
                if let Ok(path) = attachment_path(&att) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

async fn load_schedulable_members(
    db: &crate::db::Db,
    conversation_id: &str,
) -> Result<Vec<Agent>, String> {
    sqlx::query_as::<_, Agent>(
        "SELECT a.*
         FROM agents a
         JOIN conversation_members cm ON cm.agent_id = a.id
         WHERE cm.conversation_id=?
           AND a.mentionable=1
           AND a.enabled=1
         ORDER BY a.created_at",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())
}

async fn load_planner_agent(db: &crate::db::Db) -> Result<Option<Agent>, String> {
    load_system_role_agent(db, "planner").await
}

async fn load_system_role_agent(db: &crate::db::Db, kind: &str) -> Result<Option<Agent>, String> {
    let pattern = format!("%,{},%", kind);
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(system_kind, '') || ',') LIKE ?
           AND enabled=1
         ORDER BY created_at
         LIMIT 1",
    )
    .bind(pattern)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())
}

async fn load_agent(db: &crate::db::Db, agent_id: &str) -> Result<Agent, String> {
    sqlx::query_as::<_, Agent>("SELECT * FROM agents WHERE id=? AND enabled=1")
        .bind(agent_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("agent {} not found or disabled", agent_id))
}

fn asks_for_sequence(text: &str) -> bool {
    [
        "然后",
        "最后",
        "总结",
        "裁决",
        "汇总",
        "综合",
        "PRD",
        "prd",
        "ADR",
        "adr",
        "文档",
        "产出",
        "产物",
        "测试计划",
        "验收标准",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// 用户意图：把当前内容「录入系统 / 沉淀为正式需求」，而非聊一聊。命中后走需求
/// 捕获路径，让 Agent 产出可一键入流水线的 requirement_draft 草稿，而不是讨论。
fn asks_to_capture_issue(text: &str) -> bool {
    [
        "加到系统",
        "增加到系统",
        "加入系统",
        "录入系统",
        "录入需求",
        "提交需求",
        "沉淀为需求",
        "沉淀成需求",
        "提交到流水线",
        "加入流水线",
        "加到流水线",
        "登记需求",
        "记录这个需求",
        "建一个需求",
        "新建需求",
        "创建需求",
        "立项",
        // 「重提 / 重新发起」：之前草稿被拒或丢失后想重新生成一张可入流水线的草稿卡。
        // 不补这些，重提消息会落到普通对话路径，Agent 只会输出纯文本草稿、无法一键提交。
        "重新提",
        "重新发起",
        "重新录入",
        "重新登记",
        "重新生成需求",
        "再提一",
        "再次提交",
        "重提",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// 需求捕获指令：要求 Agent 先核实现状（避免重复造轮子），再以固定 JSON 结构产出
/// requirement_draft 产物。`extract_issue_draft_artifact` 会解析该 JSON，
/// `run_agent_for_step` 再补上 `t:"artifact"` 与真实 `project_id`，前端即可一键提交。
fn capture_issue_instruction(user_text: &str) -> String {
    format!(
        "用户希望把下面这条内容**录入系统、沉淀为一条正式需求**，而不是展开讨论：\n\n{}\n\n\
请按以下步骤处理：\n\
1. 若你有代码检索工具（list_files / search_code / read_file），**先检索当前项目仓库**，\
确认该需求与既有功能是否重叠，判断现状（已实现 / 部分实现 / 全新），避免提出重复造轮子的需求；\
**没有检索工具或无法核实时，必须如实说明「现状未经核实」，不得凭空臆断。**\n\
2. 用一句话给出清晰的需求标题；\n\
3. 正文写明：背景与现状、目标、范围（做 / 不做）、关键约束、验收要点；\n\
4. **最后必须输出一个 requirement_draft 产物 JSON**（用 ```json 代码块包裹），结构如下，供用户一键提交到流水线：\n\
```json\n\
{{\n\
  \"kind\": \"issue_draft\",\n\
  \"title\": \"需求标题\",\n\
  \"rows\": [[\"状态\", \"草案\"], [\"现状\", \"已实现/部分实现/全新/未核实\"], [\"类别\", \"feature\"]],\n\
  \"body\": \"需求正文（Markdown）\",\n\
  \"_meta\": {{\n\
    \"title\": \"需求标题\",\n\
    \"description\": \"完整需求描述（含现状结论）\",\n\
    \"category\": \"feature\",\n\
    \"severity\": \"medium\"\n\
  }}\n\
}}\n\
```",
        user_text.trim()
    )
}

fn asks_for_synthesis(text: &str) -> bool {
    [
        "总结", "裁决", "汇总", "综合", "结论", "建议", "最后", "评审",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn asks_for_artifact(text: &str) -> bool {
    [
        "PRD",
        "prd",
        "ADR",
        "adr",
        "文档",
        "产出",
        "产物",
        "测试计划",
        "验收标准",
        "需求说明",
        "实施方案",
        "方案文档",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// 收窄版"纯综合/收尾请求"判定：消息很短且主旨就是收口（总结/裁决/综合/结论…），
/// 而非在正常对话里顺带提到这些词。配合 last_speaking_member 一起，决定是否走
/// "只综合前文、不安排业务 Agent 新答"的空计划路径，避免日常对话被误判为伪沉默。
fn is_pure_synthesis_request(text: &str) -> bool {
    let t = text.trim();
    t.chars().count() <= 24 && asks_for_synthesis(t)
}

/// 无 @ 时的零成本相关性分诊（方向一）：按成员的 name / name_en / role / role_type
/// 与消息做大小写无关的字面匹配打分，返回得分最高的成员 id（得分>0）；平局取成员
/// 顺序靠前者。任何成员都没命中则返回 None，交由连续性 / planner / fallback 处理。
/// 纯函数、不触 LLM，等价于用户"打了角色名但忘了加 @"的软提及。
fn route_by_relevance(instruction: &str, members: &[Agent]) -> Option<String> {
    let hay = instruction.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for m in members {
        // 编码 Agent 成员（CLI 重型、只读跑仓库）只在被**显式 @** 时触发，绝不参与零成本
        // 关键词自动选人——避免日常闲聊误把昂贵 CLI 拉起来。见 [[迁移 0079]]。
        if is_code_agent_member(m) {
            continue;
        }
        let mut score = 0usize;
        for kw in [
            m.name.as_str(),
            m.name_en.as_str(),
            m.role.as_str(),
            m.role_type.as_str(),
        ] {
            let kw = kw.trim();
            // 至少 2 个字符才算关键词，避免单字/空串造成的噪声误命中。
            if kw.chars().count() >= 2 && hay.contains(&kw.to_lowercase()) {
                score += 1;
            }
        }
        if score > 0 && best.is_none_or(|(b, _)| score > b) {
            best = Some((score, m.id.as_str()));
        }
    }
    best.map(|(_, id)| id.to_string())
}

/// 找出会议室中最近一次"由某 Agent 发出"的消息作者，即上一轮发言人（方向二）。
/// 触发任务的用户消息 from_agent 为 NULL，故这里取到的是真正的上一轮 Agent。
/// 仅当该作者仍是当前可调度成员时返回，用于无 @ 追问时的对话连续性默认接话。
async fn last_speaking_member(
    db: &crate::db::Db,
    conversation_id: &str,
    members: &[Agent],
) -> Option<String> {
    let last: Option<(String,)> = sqlx::query_as(
        "SELECT from_agent FROM messages
         WHERE conversation_id=? AND from_agent IS NOT NULL
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let last = last.map(|(a,)| a)?;
    // 同理：编码 Agent 成员不作为"对话连续性默认接话人"自动续接，只在被显式 @ 时回复。
    members
        .iter()
        .find(|m| m.id == last && !is_code_agent_member(m))
        .map(|m| m.id.clone())
}

/// 该成员是否由编码 Agent（CLI）后端驱动（`agents.code_agent_id` 非空）。这类成员只读跑项目
/// 仓库作答、进程重，故只在被**显式 @mention** 或单点指定时触发，不参与自动选人/连续性接话。
fn is_code_agent_member(agent: &Agent) -> bool {
    agent
        .code_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
}

fn infer_artifact_kind(text: &str) -> &'static str {
    if text.contains("ADR") || text.contains("adr") {
        "ADR"
    } else if text.contains("测试") || text.contains("验收") {
        "测试计划"
    } else if text.contains("实施") || text.contains("方案") {
        "实施方案"
    } else if text.contains("PRD") || text.contains("prd") || text.contains("需求") {
        "PRD"
    } else {
        "文档产物"
    }
}

fn parse_artifact_payload(raw: &str, instruction: &str) -> ArtifactPayload {
    let trimmed = raw.trim();
    let parsed = serde_json::from_str::<ArtifactPayload>(trimmed)
        .ok()
        .or_else(|| {
            let start = trimmed.find('{')?;
            let end = trimmed.rfind('}')?;
            serde_json::from_str::<ArtifactPayload>(&trimmed[start..=end]).ok()
        });

    let mut artifact = parsed.unwrap_or_else(|| ArtifactPayload {
        kind: infer_artifact_kind(instruction).to_string(),
        title: default_artifact_title(instruction),
        rows: Vec::new(),
        body: raw.trim().to_string(),
    });

    if artifact.kind.trim().is_empty() {
        artifact.kind = infer_artifact_kind(instruction).to_string();
    }
    if artifact.title.trim().is_empty() {
        artifact.title = default_artifact_title(instruction);
    }
    if artifact.body.trim().is_empty() {
        artifact.body = raw.trim().to_string();
    }
    if artifact.rows.is_empty() {
        artifact.rows = vec![
            ["状态".to_string(), "草案".to_string()],
            ["来源".to_string(), "群聊讨论".to_string()],
            ["类型".to_string(), artifact.kind.clone()],
        ];
    }
    artifact
}

fn default_artifact_title(instruction: &str) -> String {
    let mut title = instruction
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect::<String>()
        .trim()
        .to_string();
    if title.is_empty() {
        title = "群聊文档产物".to_string();
    }
    title
}

fn parse_plan_json(raw: &str) -> Result<ConversationPlan, String> {
    let trimmed = raw.trim();
    if let Ok(plan) = serde_json::from_str::<ConversationPlan>(trimmed) {
        return Ok(plan);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| "planner 未输出 JSON".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "planner JSON 不完整".to_string())?;
    serde_json::from_str::<ConversationPlan>(&trimmed[start..=end])
        .map_err(|e| format!("planner JSON 解析失败: {}", e))
}

/// 从 LLM 输出中提取 requirement_draft artifact JSON block。
/// 若找到，返回 (清理后的文本, Some(artifact值))，否则返回 (原文本, None)。
fn extract_issue_draft_artifact(text: &str) -> (String, Option<serde_json::Value>) {
    if !text.contains("issue_draft") && !text.contains("requirement_draft") {
        return (text.to_string(), None);
    }

    // 遍历文本，寻找以 '{' 开始的平衡 JSON 对象
    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] != b'{' {
            pos += 1;
            continue;
        }
        // 找到平衡的 JSON 对象
        let mut depth = 0i32;
        let mut end = None;
        for (i, &b) in bytes[pos..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(pos + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end_pos) = end {
            let candidate = &text[pos..end_pos];
            if candidate.contains("issue_draft") || candidate.contains("requirement_draft") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
                    if matches!(val.get("kind").and_then(|k| k.as_str()), Some("issue_draft") | Some("requirement_draft")) {
                        // 移除 JSON 及周围的 markdown 代码围栏
                        let before = text[..pos]
                            .trim_end()
                            .trim_end_matches("```json")
                            .trim_end_matches("```")
                            .trim_end()
                            .to_string();
                        let after = text[end_pos..]
                            .trim_start()
                            .trim_start_matches("```")
                            .trim_start()
                            .to_string();
                        let clean = format!("{} {}", before, after).trim().to_string();
                        return (clean, Some(val));
                    }
                }
            }
            pos = end_pos;
        } else {
            break;
        }
    }

    (text.to_string(), None)
}

fn emit_task_update(app: &AppHandle, conversation_id: &str, task_id: &str, status: &str) {
    event::emit(
        app,
        event::AppEvent::ConversationTaskUpdated {
            conversation_id: conversation_id.to_string(),
            task_id: task_id.to_string(),
            status: status.to_string(),
        },
    );
    info!("[orchestration] task {} status={}", task_id, status);
}
