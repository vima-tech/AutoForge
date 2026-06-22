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

/// 一页需求 + 该筛选条件下的总数。前端「全量需求总账」滚动动态加载用，
/// 避免需求量增长后一次性把全部需求拉进内存/渲染。
#[derive(serde::Serialize)]
pub struct IssuePage {
    pub items: Vec<Issue>,
    pub total: i64,
}

/// 分页查询需求：按项目 + 状态 + 关键字（标题/编号）过滤，按 updated_at 倒序。
/// status 为空或 "all" 表示不过滤状态；search 为空表示不过滤关键字。
/// exclude_merged 为 true 时排除「已合并」需求（与功能审计页「显示已合并需求」开关共享，默认隐藏）；
/// 当显式按 merged 状态筛选时该排除自然失效（status 优先）。
#[tauri::command]
pub async fn list_issues_page(
    project_id: Option<String>,
    status: Option<String>,
    search: Option<String>,
    exclude_merged: Option<bool>,
    limit: i64,
    offset: i64,
    sort_asc: Option<bool>,
    state: State<'_, AppState>,
) -> Result<IssuePage, String> {
    use sqlx::{QueryBuilder, Sqlite};
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let status = status.filter(|s| !s.is_empty() && s != "all");
    // 仅当未显式筛选 merged 时才生效（显式选 merged 说明用户主动想看，不再隐藏）。
    let exclude_merged =
        exclude_merged.unwrap_or(false) && status.as_deref() != Some("merged");
    // LIKE 通配串作为局部变量，确保在两次 query 构建期间存活。
    let like = search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{}%", s));

    let total: i64 = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        qb.build_query_scalar()
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let items = {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        // 统一按创建时间排序（updated_at 会随执行/重分析变动，跨页不稳定）。
        // 默认正序：旧需求置前，避免问题积压长期无人处理。
        qb.push(if sort_asc.unwrap_or(true) {
            " ORDER BY created_at ASC LIMIT "
        } else {
            " ORDER BY created_at DESC LIMIT "
        })
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
        qb.build_query_as::<Issue>()
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(IssuePage { items, total })
}

/// 某项目下出现过的所有状态（去重），用于总账的状态筛选 chip——
/// 不依赖已加载的那一页，仍能列出全部可筛选状态。
#[tauri::command]
pub async fn list_issue_statuses(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT DISTINCT status FROM issues WHERE project_id=?")
        .bind(&project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 按状态集合取需求（完整字段），用于「需求审核」队列等有界子集，
/// 替代为拿少量在产需求而全量加载 issues。
#[tauri::command]
pub async fn list_issues_by_statuses(
    project_id: String,
    statuses: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if statuses.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE project_id = ");
    qb.push_bind(&project_id).push(" AND status IN (");
    let mut sep = qb.separated(", ");
    for s in &statuses {
        sep.push_bind(s);
    }
    qb.push(") ORDER BY created_at DESC");
    qb.build_query_as::<Issue>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 需求标题（轻量），按 id 批量取，用于变更请求列表解析标题，
/// 无需把全部需求载入内存。
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct IssueTitle {
    pub id: String,
    pub title: String,
}

#[tauri::command]
pub async fn list_issue_titles(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<IssueTitle>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT id, title FROM issues WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id);
    }
    qb.push(")");
    qb.build_query_as::<IssueTitle>()
        .fetch_all(&state.db)
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
/// job under a UNIQUE idempotency key (`analysis:<id>:retry:<uuid>`). The stable
/// `analysis:<id>` key would silently de-duplicate against an already-`completed`
/// job row — `enqueue` only re-dispatches a `failed` row — leaving the issue parked
/// at `pending_analysis` forever. A unique key always dispatches, so a transient LLM
/// failure (or a re-analysis of an already-analyzed issue) recovers in one click.
/// The status-guarded UPDATE above already idempotently blocks a double-click.
#[tauri::command]
pub async fn retry_analysis(
    issue_id: String,
    _app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let updated = sqlx::query(
        "UPDATE issues SET status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('analysis_failed', 'pending_issue_review')",
    )
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「分析失败」或「待需求审核」状态的需求可重新分析".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 需求审核 补充意见重评：管理员在「待需求审核」阶段提交补充意见，需求带着该意见
/// 重新分析，再回到需求审核。意见落库到 issues.review_feedback（一次性，被分析任务
/// 消费后清空）。意见为人工输入，入库前过 has_obvious_injection 防注入。
#[tauri::command]
pub async fn reanalyze_with_feedback(
    issue_id: String,
    feedback: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let feedback = feedback.trim().to_string();
    if feedback.is_empty() {
        return Err("补充意见不能为空".to_string());
    }
    if crate::core::security::has_obvious_injection(&feedback) {
        return Err("补充意见包含可疑指令，已拦截".to_string());
    }
    // 仅「待需求审核」或「分析失败」状态可带意见重评，避免影响在产/已完结需求。
    let updated = sqlx::query(
        "UPDATE issues SET review_feedback=?, status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('pending_issue_review', 'analysis_failed')",
    )
    .bind(&feedback)
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「待需求审核」或「分析失败」状态的需求可提交补充意见重新评估".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        // 唯一 key：稳定的 analysis:<id> 会被 enqueue 按已 completed 的旧 job 行去重而
        // 不再派发，需求会永远停在 pending_analysis。带意见重评必须真正重跑。
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
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
