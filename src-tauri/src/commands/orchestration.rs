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

    let trigger_exists: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM messages WHERE id=? AND conversation_id=?",
    )
    .bind(&payload.trigger_message_id)
    .bind(&payload.conversation_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if trigger_exists.is_none() {
        return Err("触发消息不存在或不属于当前对话".to_string());
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
    tauri::async_runtime::spawn(async move {
        if let Err(e) = execute_conversation_task(db.clone(), app_for_task.clone(), conversation, payload.clone(), task_id.clone()).await {
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
    });

    Ok(task)
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

async fn execute_conversation_task(
    db: crate::db::Db,
    app: AppHandle,
    conversation: Conversation,
    payload: StartConversationTask,
    task_id: String,
) -> Result<(), String> {
    let planner = load_planner_agent(&db).await?;
    let window_size = payload.window_size.unwrap_or(30).clamp(5, 100);
    let snapshot = build_context_snapshot(&db, &payload.conversation_id, window_size).await?;
    let members = load_schedulable_members(&db, &payload.conversation_id).await?;

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

    sqlx::query(
        "UPDATE conversation_tasks SET planner_agent_id=?, plan_json=? WHERE id=?",
    )
    .bind(planner.as_ref().map(|a| a.id.as_str()))
    .bind(&plan_json)
    .bind(&task_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;

    emit_task_update(&app, &payload.conversation_id, &task_id, "running");

    let mut accumulated = String::new();
    let mut any_failed = false;
    for (index, step) in plan.steps.iter().enumerate() {
        let step_id = Uuid::new_v4().to_string();
        let agent_ids_json = serde_json::to_string(&step.agents).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO conversation_task_steps
             (id, task_id, step_index, step_type, agent_ids_json, instruction, status, started_at)
             VALUES (?, ?, ?, ?, ?, ?, 'running', datetime('now'))",
        )
        .bind(&step_id)
        .bind(&task_id)
        .bind(index as i64)
        .bind(&step.step_type)
        .bind(&agent_ids_json)
        .bind(&step.instruction)
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?;

        let mut calls = Vec::new();
        for agent_id in &step.agents {
            calls.push(run_agent_for_step(
                db.clone(),
                app.clone(),
                task_id.clone(),
                step_id.clone(),
                payload.conversation_id.clone(),
                agent_id.clone(),
                step.instruction.clone(),
                snapshot.clone(),
                accumulated.clone(),
            ));
        }

        let outcomes = join_all(calls).await;
        let mut step_failed = false;
        for outcome in outcomes {
            match outcome {
                Ok(outcome) => {
                    if !outcome.ok {
                        step_failed = true;
                    }
                    accumulated.push_str(&format!(
                        "\n\n[{} / {}]\n{}",
                        outcome.agent_name, outcome.agent_id, outcome.text
                    ));
                }
                Err(e) => {
                    step_failed = true;
                    accumulated.push_str(&format!("\n\n[执行失败]\n{}", e));
                }
            }
        }

        any_failed |= step_failed;
        sqlx::query(
            "UPDATE conversation_task_steps
             SET status=?, completed_at=datetime('now'), error=?
             WHERE id=?",
        )
        .bind(if step_failed { "failed" } else { "completed" })
        .bind(if step_failed { Some("部分 Agent 执行失败") } else { None::<&str> })
        .bind(&step_id)
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?;
    }

    let final_status = if any_failed { "failed" } else { "completed" };
    sqlx::query(
        "UPDATE conversation_tasks
         SET status=?, completed_at=datetime('now'), error=?
         WHERE id=?",
    )
    .bind(final_status)
    .bind(if any_failed { Some("部分步骤执行失败") } else { None::<&str> })
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

    let mentioned: Vec<String> = mentioned_agent_ids
        .iter()
        .filter(|id| members.iter().any(|a| &a.id == *id))
        .cloned()
        .collect();
    let needs_planner = mentioned.is_empty() || asks_for_sequence(instruction);
    if !needs_planner {
        return Ok(ConversationPlan {
            steps: vec![ConversationPlanStep {
                step_type: if mentioned.len() > 1 { "parallel" } else { "single" }.to_string(),
                agents: mentioned,
                instruction: instruction.to_string(),
            }],
        });
    }

    if let Some(planner) = planner {
        match ask_planner(db, planner, instruction, &mentioned, members, snapshot).await {
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

    let system_prompt = if planner.system_prompt.trim().is_empty() {
        None
    } else {
        Some(planner.system_prompt.as_str())
    };
    let raw = crate::agents::llm::run_agent_text(
        db,
        planner,
        &prompt,
        system_prompt,
        &[],
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
        members.iter().take(1).map(|a| a.id.clone()).collect::<Vec<_>>()
    } else {
        mentioned_agent_ids
    };
    if agents.is_empty() {
        return Err("当前对话没有可调度的 Agent".to_string());
    }
    Ok(ConversationPlan {
        steps: vec![ConversationPlanStep {
            step_type: if agents.len() > 1 { "parallel" } else { "single" }.to_string(),
            agents,
            instruction: instruction.to_string(),
        }],
    })
}

async fn run_agent_for_step(
    db: crate::db::Db,
    app: AppHandle,
    task_id: String,
    step_id: String,
    conversation_id: String,
    agent_id: String,
    instruction: String,
    snapshot: String,
    accumulated: String,
) -> Result<AgentOutcome, String> {
    let run_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO conversation_task_runs
         (id, task_id, step_id, agent_id, status, started_at)
         VALUES (?, ?, ?, ?, 'running', datetime('now'))",
    )
    .bind(&run_id)
    .bind(&task_id)
    .bind(&step_id)
    .bind(&agent_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;

    let agent = load_agent(&db, &agent_id).await?;
    let prompt = format!(
        "以下是群聊对话快照：\n{}\n\n前置 Agent 发言：\n{}\n\n当前任务：\n{}\n\n请以 {} 的身份在群聊中直接回复。保持观点明确，必要时输出结构化 Markdown。",
        snapshot,
        if accumulated.trim().is_empty() { "无" } else { &accumulated },
        instruction,
        agent.name
    );
    let system_prompt = if agent.system_prompt.trim().is_empty() {
        None
    } else {
        Some(agent.system_prompt.as_str())
    };
    let result = crate::agents::llm::run_agent_text(
        &db,
        &agent,
        &prompt,
        system_prompt,
        &[],
    )
    .await;

    let (ok, text, error) = match result {
        Ok(text) => (true, text, None),
        Err(e) => {
            let msg = format!("[系统错误: {}]", e);
            (false, msg.clone(), Some(e.to_string()))
        }
    };

    let content_json = serde_json::json!([{ "t": "md", "md": text.clone() }]).to_string();
    let message_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(&conversation_id)
    .bind(&agent_id)
    .bind(&content_json)
    .execute(&db)
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
    .bind(&run_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?;

    event::emit(
        &app,
        event::AppEvent::MessageReceived {
            conversation_id,
            message_id,
        },
    );

    Ok(AgentOutcome {
        agent_id,
        agent_name: agent.name,
        ok,
        text,
    })
}

async fn build_context_snapshot(
    db: &crate::db::Db,
    conversation_id: &str,
    limit: i64,
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

    let agent_rows = sqlx::query_as::<_, Agent>("SELECT * FROM agents")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    let agent_names: HashMap<String, String> = agent_rows
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();

    let mut parts = Vec::new();
    for msg in candidates.iter().rev().filter(|m| !m.excluded_from_context) {
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
                let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("artifact");
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
                        Ok(Some(content)) => parts.push(format!("[文件内容 - {}]\n```\n{}\n```", name, content)),
                        Ok(None) => parts.push(format!("[附件: {}]", name)),
                        Err(e) => parts.push(format!("[附件: {} 读取失败: {}]", name, e)),
                    }
                }
            }
            Some("image") => {
                let label = block.get("label").and_then(|v| v.as_str()).unwrap_or("图片");
                parts.push(format!("[图片: {}]", label));
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
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "文件不是有效的 UTF-8 文本".to_string())?;
    const MAX_CHARS: usize = 50_000;
    if text.chars().count() > MAX_CHARS {
        Ok(Some(text.chars().take(MAX_CHARS).collect::<String>()))
    } else {
        Ok(Some(text.to_string()))
    }
}

fn attachment_path(attachment: &ConversationAttachment) -> Result<PathBuf, String> {
    let rel = Path::new(&attachment.rel_path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("附件路径无效".to_string());
    }
    Ok(PathBuf::from(crate::state::attachments_base()).join(rel))
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
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(system_kind, '') || ',') LIKE '%,planner,%'
         ORDER BY created_at
         LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())
}

async fn load_agent(db: &crate::db::Db, agent_id: &str) -> Result<Agent, String> {
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents WHERE id=? AND enabled=1",
    )
    .bind(agent_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("agent {} not found or disabled", agent_id))
}

fn asks_for_sequence(text: &str) -> bool {
    ["然后", "最后", "总结", "裁决", "汇总", "综合", "PRD", "文档", "产出"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn parse_plan_json(raw: &str) -> Result<ConversationPlan, String> {
    let trimmed = raw.trim();
    if let Ok(plan) = serde_json::from_str::<ConversationPlan>(trimmed) {
        return Ok(plan);
    }
    let start = trimmed.find('{').ok_or_else(|| "planner 未输出 JSON".to_string())?;
    let end = trimmed.rfind('}').ok_or_else(|| "planner JSON 不完整".to_string())?;
    serde_json::from_str::<ConversationPlan>(&trimmed[start..=end])
        .map_err(|e| format!("planner JSON 解析失败: {}", e))
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
