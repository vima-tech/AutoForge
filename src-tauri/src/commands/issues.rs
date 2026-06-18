use crate::intake::{gateway, IntakeMode, IntakePayload};
use crate::models::issue::{CreateIssue, Issue, IssueAnalysis};
use crate::models::job::JobPayload;
use crate::state::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn list_issues(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    if let Some(pid) = project_id {
        sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, Issue>("SELECT * FROM issues ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_issue(id: String, state: State<'_, AppState>) -> Result<Option<Issue>, String> {
    sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_issue_analysis(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Option<IssueAnalysis>, String> {
    sqlx::query_as::<_, IssueAnalysis>("SELECT * FROM issue_analyses WHERE issue_id=?")
        .bind(&issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn submit_issue(
    payload: CreateIssue,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Issue, String> {
    let mode = IntakeMode::from_opt(payload.mode.as_deref());
    let has_bug = payload.repro_steps.is_some()
        || payload.environment.is_some()
        || payload.expected.is_some()
        || payload.actual.is_some();
    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: payload.source_type.unwrap_or_else(|| "manual".to_string()),
        source_ref: payload.source_ref,
    };
    let issue = gateway::receive(&state.db, &state.job_tx, &app, intake, mode).await?;

    // Bug 载体字段单独落库（保持 IntakePayload 与六通道不变）。
    if has_bug {
        sqlx::query(
            "UPDATE issues SET repro_steps=?, environment=?, expected=?, actual=? WHERE id=?",
        )
        .bind(&payload.repro_steps)
        .bind(&payload.environment)
        .bind(&payload.expected)
        .bind(&payload.actual)
        .bind(&issue.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        return sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&issue.id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }
    Ok(issue)
}

/// Re-run requirement analysis for an issue whose analysis failed (or is stuck at
/// review 1). Resets the issue to `pending_analysis` and re-enqueues the analysis
/// job. Re-enqueuing reuses the existing idempotency row (incrementing its attempt
/// counter) and still re-dispatches the work, so a transient LLM failure recovers
/// in one click instead of leaving the issue on a dead-end.
#[tauri::command]
pub async fn retry_analysis(
    issue_id: String,
    _app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let updated = sqlx::query(
        "UPDATE issues SET status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('analysis_failed', 'pending_review_1')",
    )
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「分析失败」或「待审核 1」状态的需求可重新分析".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        &format!("analysis:{}", issue_id),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 列出某 CR 的测试遥测记录（review_2 合并前自动测试结果）。
#[tauri::command]
pub async fn list_cr_test_runs(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::test_run::CrTestRun>, String> {
    sqlx::query_as::<_, crate::models::test_run::CrTestRun>(
        "SELECT * FROM cr_test_runs WHERE cr_id=? ORDER BY run_at DESC",
    )
    .bind(&cr_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 人审改 AI 生成的验收标准（acceptance_json，JSON 数组字符串）。
#[tauri::command]
pub async fn update_issue_acceptance(
    issue_id: String,
    acceptance_json: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 校验是合法 JSON，避免写入脏数据破坏后续解析。
    serde_json::from_str::<serde_json::Value>(&acceptance_json)
        .map_err(|e| format!("验收标准 JSON 非法：{}", e))?;
    sqlx::query("UPDATE issues SET acceptance_json=?, updated_at=datetime('now') WHERE id=?")
        .bind(&acceptance_json)
        .bind(&issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
