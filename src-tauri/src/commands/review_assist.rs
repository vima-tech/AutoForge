//! 审核辅助：代码预审摘要（code_reviewer）+ 发布说明（release_notes）。
//!
//! 两者都是「按需生成 + 落 agent_outputs + 可回读」的轻量 AI 辅助，**不触碰**关键的
//! 执行/合并/审核流水线（避免在核心路径引入风险）。生成由操作者在审核页主动触发；
//! 结果存 `agent_outputs`（target_kind=change_request, target_id=cr_id），可重复生成（取最新一条）。
//!
//! 解耦：命令体只做「取 state → 调下层纯逻辑 → 返回」，实际逻辑放本文件的普通 async fn，
//! 不在命令体堆 sqlx/业务。

use crate::db::Db;
use crate::state::AppState;
use tauri::State;

/// 取某 CR 关联需求的标题与描述（供生成时提供需求上下文）。缺失则返回空串，不阻断。
async fn cr_issue_brief(db: &Db, cr_id: &str) -> (String, String) {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT i.title, i.description FROM change_requests c
         JOIN issues i ON i.id = c.issue_id WHERE c.id = ?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    row.unwrap_or_default()
}

/// 取某 CR 的项目 id（用于召回与落库归属）。
async fn cr_project_id(db: &Db, cr_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT project_id FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// 读取某 CR 最新一条指定角色的产出原文（raw）。无则 None。
async fn latest_output(db: &Db, role: &str, cr_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT raw FROM agent_outputs
         WHERE role=? AND target_kind='change_request' AND target_id=?
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(role)
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// diff 截断上限（喂模型前，避免超长 diff 撑爆上下文）。保留头部（变更摘要通常在前）。
const MAX_DIFF_CHARS: usize = 48_000;

fn clip_diff(diff: &str) -> String {
    if diff.chars().count() <= MAX_DIFF_CHARS {
        return diff.to_string();
    }
    let head: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    format!("{head}\n\n…[diff 过长已截断，仅展示前 {MAX_DIFF_CHARS} 字符]")
}

async fn generate_for_role(
    db: &Db,
    cr_id: &str,
    role: &str,
    instruction_label: &str,
) -> Result<String, String> {
    let diff = crate::commands::change_requests::load_cr_diff(db, cr_id).await?;
    if diff.trim().is_empty() {
        return Err("该变更暂无可分析的代码 diff（worktree 已清理或为空改动）".to_string());
    }
    let (title, desc) = cr_issue_brief(db, cr_id).await;
    let project_id = cr_project_id(db, cr_id).await;
    let prompt = format!(
        "{label}\n\n## 需求\n标题：{title}\n描述：{desc}\n\n## 代码 diff\n```diff\n{diff}\n```",
        label = instruction_label,
        title = title,
        desc = desc,
        diff = clip_diff(&diff),
    );
    let (raw, trace_id) = crate::agents::llm::run_system_role_text_traced(
        db,
        role,
        &prompt,
        None,
        project_id.as_deref(),
        Some(&title),
    )
    .await
    .map_err(|e| e.to_string())?;

    // 落库（复用 agent_outputs；schema_version 标 1.0，raw 即可回读渲染）。
    crate::agents::schema::record(
        db,
        role,
        "1.0",
        "change_request",
        cr_id,
        project_id.as_deref(),
        trace_id.as_deref(),
        "ok",
        &raw,
        &raw,
    )
    .await;
    Ok(raw)
}

/// 生成 CR 的 AI 预审摘要（Markdown）。供「代码审核」页操作者主动触发，减负人审。
#[tauri::command]
pub async fn generate_code_review_summary(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    generate_for_role(
        &state.db,
        &cr_id,
        "code_reviewer",
        "请对下面这次代码变更生成预审摘要，帮助审核者快速抓住重点。",
    )
    .await
}

/// 读取已生成的最新 AI 预审摘要（无则空串）。
#[tauri::command]
pub async fn get_code_review_summary(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    Ok(latest_output(&state.db, "code_reviewer", &cr_id).await.unwrap_or_default())
}

/// 生成 CR 的发布说明（JSON：kind/headline/body 的原文字符串）。
#[tauri::command]
pub async fn generate_release_notes(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    generate_for_role(
        &state.db,
        &cr_id,
        "release_notes",
        "请依据下面的需求与代码 diff 生成面向用户的变更说明（changelog 条目）。",
    )
    .await
}

/// 读取已生成的最新发布说明（无则空串）。
#[tauri::command]
pub async fn get_release_notes(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    Ok(latest_output(&state.db, "release_notes", &cr_id).await.unwrap_or_default())
}
