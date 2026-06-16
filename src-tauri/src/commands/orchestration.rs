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
        let (outcomes, step_failed) = run_plan_step(
            &db,
            &app,
            &task_id,
            &payload.conversation_id,
            next_step_index,
            &step.step_type,
            &step.agents,
            &step.instruction,
            &snapshot,
            &accumulated,
            &roster,
            conversation.project_id.as_deref(),
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
        let (outcomes, step_failed) = run_plan_step(
            &db,
            &app,
            &task_id,
            &payload.conversation_id,
            next_step_index,
            step_type,
            &to_run,
            instruction,
            &snapshot,
            &accumulated,
            &roster,
            conversation.project_id.as_deref(),
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
            let outcome = run_summarizer(
                &db,
                &app,
                &payload.conversation_id,
                &summarizer,
                &payload.instruction,
                &snapshot,
                &accumulated,
                conversation.project_id.as_deref(),
            )
            .await?;
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
            let outcome = run_doc_writer(
                &db,
                &app,
                &payload.conversation_id,
                &doc_writer,
                &payload.instruction,
                &snapshot,
                &accumulated,
                conversation.project_id.as_deref(),
            )
            .await?;
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

    let mentioned: Vec<String> = mentioned_agent_ids
        .iter()
        .filter(|id| members.iter().any(|a| &a.id == *id))
        .cloned()
        .collect();

    // Pure synthesis request with no @mentions: skip Planner and all business-agent steps.
    // Returning an empty plan makes plan_has_final_single=false, so the post-plan
    // summarizer hook fires directly with the conversation snapshot as context.
    if mentioned.is_empty() && asks_for_synthesis(instruction) && !asks_for_artifact(instruction) {
        return Ok(ConversationPlan { steps: vec![] });
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
        members
            .iter()
            .take(1)
            .map(|a| a.id.clone())
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
async fn run_plan_step(
    db: &crate::db::Db,
    app: &AppHandle,
    task_id: &str,
    conversation_id: &str,
    step_index: usize,
    step_type: &str,
    agents: &[String],
    instruction: &str,
    snapshot: &str,
    accumulated: &str,
    roster: &str,
    project_id: Option<&str>,
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
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    let mut calls = Vec::new();
    for agent_id in agents {
        calls.push(run_agent_for_step(
            db.clone(),
            app.clone(),
            task_id.to_string(),
            step_id.clone(),
            conversation_id.to_string(),
            agent_id.clone(),
            instruction.to_string(),
            snapshot.to_string(),
            accumulated.to_string(),
            roster.to_string(),
            project_id.map(str::to_string),
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
    .execute(db)
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
    db: crate::db::Db,
    app: AppHandle,
    task_id: String,
    step_id: String,
    conversation_id: String,
    agent_id: String,
    instruction: String,
    snapshot: String,
    accumulated: String,
    roster: String,
    project_id: Option<String>,
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
    let roster_section = if roster.trim().is_empty() {
        String::new()
    } else {
        format!(
            "群聊成员名单（了解在场成员；话题相关时可 @名字 点名协作）：\n{}\n\n",
            roster
        )
    };
    let prompt = format!(
        "{}以下是群聊对话快照：\n{}\n\n前置 Agent 发言：\n{}\n\n当前任务：\n{}\n\n请以 {} 的身份在群聊中直接回复。保持观点明确，必要时输出结构化 Markdown。\n优先自己把问题答完，不必为了协作而刻意 @ 别人；但当某部分确实更适合其他成员的专长、或你想就分歧点邀请其表态时，可以自然地用 @对方名字 点名（仅 @ 名单中的成员，且只 @ 与当前话题真正相关的成员）。不要为了凑发言或客套而 @ 无关成员。",
        roster_section,
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
    // 群聊步骤 Agent 的工具集：内置工具(capabilities 白名单) + 代码扫描(项目仓库) + 勾选的 MCP server 工具。
    let tool_ctx = crate::agents::tools::ToolContext::resolve(&db, project_id.as_deref()).await;
    let registry =
        crate::agents::tools::build_registry_for_agent(&db, &agent, &tool_ctx).await;
    // 把「LLM 调用 + 解析 <write-file> + 落盘」包进同一个 trace run：写文件以 tool span
    // 挂在与本次 Agent 调用相同的 trace 下，链路追踪里即可审计 Agent 写了哪些工作区文件。
    let (ok, text, text_after_writes, error, write_blocks) =
        crate::core::trace::scope_run(&db, &agent, async {
            let result = crate::agents::llm::run_agent_text_with_tools(
                &db, &agent, &prompt, system_prompt, &[], &registry,
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
                crate::commands::workspace::execute_agent_writes(&db, &conversation_id, file_writes)
                    .await;
            (ok, text, text_after_writes, error, write_blocks)
        })
        .await;

    // 检测 LLM 输出中是否嵌入了 requirement_draft artifact JSON
    let (clean_text, draft_artifact) = extract_requirement_draft_artifact(&text_after_writes);
    let mut blocks = vec![serde_json::json!({ "t": "md", "md": clean_text })];
    if let Some(artifact) = draft_artifact {
        blocks.push(artifact);
    }
    for wb in write_blocks {
        blocks.push(wb);
    }
    let content_json = serde_json::to_string(&blocks).unwrap_or_default();

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

async fn run_summarizer(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    agent: &Agent,
    instruction: &str,
    snapshot: &str,
    accumulated: &str,
    project_id: Option<&str>,
) -> Result<AgentOutcome, String> {
    let prompt = format!(
        "以下是群聊对话快照：\n{}\n\n本轮 Agent 发言：\n{}\n\n用户原始请求：\n{}\n\n请作为群聊总结器输出最终结论。要求：\n- 综合各方观点，不重复完整原文。\n- 如果用户要求裁决，明确给出裁决和理由。\n- 输出后续行动建议。\n- 使用结构化 Markdown。",
        snapshot,
        if accumulated.trim().is_empty() { "无" } else { accumulated },
        instruction
    );
    let fallback_system =
        "你是 AutoForge 的系统总结器，负责把多 Agent 讨论压缩成清晰、可执行、可追溯的结论。";
    run_system_agent_markdown(
        db, app, conversation_id, agent, "summarizer", &prompt, fallback_system,
        project_id, Some(instruction),
    )
    .await
}

async fn run_doc_writer(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    agent: &Agent,
    instruction: &str,
    snapshot: &str,
    accumulated: &str,
    project_id: Option<&str>,
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
        snapshot,
        if accumulated.trim().is_empty() {
            "无"
        } else {
            accumulated
        },
        instruction,
        default_kind
    );
    let fallback_system =
        "你是 AutoForge 的系统文档生成器，负责把群聊讨论沉淀为可引用、可迭代的文档产物。";
    let (ok, raw) = run_system_agent_text(
        db, agent, "doc_writer", &prompt, fallback_system, project_id, Some(instruction),
    )
    .await;
    if !ok {
        let message_id =
            insert_agent_markdown_message(db, conversation_id, &agent.id, &raw).await?;
        event::emit(
            app,
            event::AppEvent::MessageReceived {
                conversation_id: conversation_id.to_string(),
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
    insert_agent_artifact_message(db, app, conversation_id, &agent.id, artifact).await?;

    Ok(AgentOutcome {
        agent_id: agent.id.clone(),
        agent_name: agent.name.clone(),
        ok,
        text: if ok { text_for_trace } else { raw },
    })
}

async fn run_system_agent_markdown(
    db: &crate::db::Db,
    app: &AppHandle,
    conversation_id: &str,
    agent: &Agent,
    kind: &str,
    prompt: &str,
    fallback_system_prompt: &str,
    project_id: Option<&str>,
    recall_key: Option<&str>,
) -> Result<AgentOutcome, String> {
    let (ok, text) =
        run_system_agent_text(db, agent, kind, prompt, fallback_system_prompt, project_id, recall_key).await;
    let message_id = insert_agent_markdown_message(db, conversation_id, &agent.id, &text).await?;
    event::emit(
        app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.to_string(),
            message_id,
        },
    );
    Ok(AgentOutcome {
        agent_id: agent.id.clone(),
        agent_name: agent.name.clone(),
        ok,
        text,
    })
}

async fn run_system_agent_text(
    db: &crate::db::Db,
    agent: &Agent,
    kind: &str,
    prompt: &str,
    fallback_system_prompt: &str,
    project_id: Option<&str>,
    recall_key: Option<&str>,
) -> (bool, String) {
    // 用注册表内置提示词（按 prompt_mode）+ Innate 召回，让群聊系统角色随经验越用越好。
    let system_prompt = crate::agents::llm::build_role_system_prompt(
        agent,
        Some(kind),
        Some(fallback_system_prompt),
        project_id,
        recall_key,
    )
    .await;
    // 系统角色也可按需用工具（代码扫描/web_search）：注册表按 capabilities 白名单 + 项目绑定装配；
    // 为空（未开启工具/无项目）时 run_agent_text_with_tools 自动回退到无工具单轮，行为不变。
    let tool_ctx = crate::agents::tools::ToolContext::resolve(db, project_id).await;
    let registry = crate::agents::tools::build_registry_for_agent(db, agent, &tool_ctx).await;
    match crate::agents::llm::run_agent_text_with_tools(
        db, agent, prompt, system_prompt.as_deref(), &[], &registry,
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
        db, &compressor, "context_compressor", &prompt, fallback_system, project_id, Some(&source),
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
    let message_id =
        insert_agent_markdown_message(db, conversation_id, &compressor.id, &markdown).await?;
    for msg in to_compress {
        sqlx::query("UPDATE messages SET excluded_from_context=1 WHERE id=?")
            .bind(&msg.id)
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
    }
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
fn extract_requirement_draft_artifact(text: &str) -> (String, Option<serde_json::Value>) {
    if !text.contains("requirement_draft") {
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
            if candidate.contains("requirement_draft") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
                    if val.get("kind").and_then(|k| k.as_str()) == Some("requirement_draft") {
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
