use crate::agents::local_claude;
use crate::models::{
    admin_decision::AdminDecision,
    preview::PreviewEnvironment,
    test_session::{ScanFinding, TestSession},
};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;
use tracing::{debug, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemHealth {
    pub status: String,
    pub db_ok: bool,
    pub claude_auth: bool,
    pub version: String,
    pub active_slots: usize,
    pub max_slots: usize,
    pub total_slot_capacity: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SlotOccupant {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectSlotStats {
    pub project_id: String,
    pub project_name: String,
    pub project_status: String,
    pub active_slots: usize,
    pub max_slots: usize,
    pub executing_slots: usize,
    pub pending_review_slots: usize,
    pub occupants: Vec<SlotOccupant>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectPipelineStats {
    pub project_id: String,
    pub project_name: String,
    pub project_status: String,
    pub pending_analysis: i64,
    pub pending_review_1: i64,
    pub executing: i64,
    pub pending_review_2: i64,
    pub merged: i64,
    pub rejected: i64,
    pub total_issues: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineStats {
    pub pending_analysis: i64,
    pub pending_review_1: i64,
    pub executing: i64,
    pub pending_review_2: i64,
    pub merged: i64,
    pub rejected: i64,
    pub total_issues: i64,
    pub active_projects: i64,
    pub active_slots: usize,
    pub max_slots: usize,
    pub total_slot_capacity: usize,
    pub pending_review_slots: usize,
    pub pause_threshold: usize,
    pub stage: String,
    pub executing_cr_ids: Vec<String>,
    pub project_slots: Vec<ProjectSlotStats>,
    pub project_pipelines: Vec<ProjectPipelineStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateConcurrencyConfig {
    pub max_slots: Option<usize>,
    pub pause_threshold: Option<usize>,
    pub queue_strategy: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    pub active_slots: usize,
    pub max_slots: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
    pub queue_strategy: String,
}

pub async fn load_concurrency_settings(
    db: &crate::db::Db,
) -> Result<(usize, usize, String), String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings
         WHERE key IN ('concurrency.max_slots', 'concurrency.pause_threshold', 'concurrency.queue_strategy')",
    )
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    let mut max_slots = 5;
    let mut pause_threshold = 20;
    let mut queue_strategy = "priority".to_string();

    for (key, value) in rows {
        match key.as_str() {
            "concurrency.max_slots" => {
                max_slots = value.parse::<usize>().unwrap_or(5).max(1);
            }
            "concurrency.pause_threshold" => {
                pause_threshold = value.parse::<usize>().unwrap_or(20).max(1);
            }
            "concurrency.queue_strategy" => {
                queue_strategy = match value.as_str() {
                    "fifo" | "priority" | "oldest" => value,
                    _ => "priority".to_string(),
                };
            }
            _ => {}
        }
    }

    Ok((max_slots, pause_threshold, queue_strategy))
}

#[tauri::command]
pub async fn system_health(state: State<'_, AppState>) -> Result<SystemHealth, String> {
    let t0 = std::time::Instant::now();
    debug!("[cmd] system_health start");
    // Check DB
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    // Auth check is intentionally NOT done here.
    // Spawning the claude Electron subprocess from within a WebKitGTK process
    // delivers SIGTRAP to the parent via kill(getppid(), SIGTRAP), which
    // triggers a NeedDebuggerBreak trap that permanently freezes the GTK event
    // loop and drops all subsequent Tauri IPC calls.
    // Use the dedicated `check_claude_auth` command for a one-shot lazy check.
    let claude_auth = true;

    let pipeline_status = state.concurrency.status();
    let (executing,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='executing'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((pipeline_status.active_slots as i64,));
    let (pending_review,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='pending_review_2'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((pipeline_status.pending_review as i64,));
    let (active_projects,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status='active'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((1,));
    let stage = if pending_review as usize >= pipeline_status.pause_threshold {
        "paused".to_string()
    } else if pending_review as usize >= pipeline_status.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    info!("[cmd] system_health done in {:?}", t0.elapsed());
    Ok(SystemHealth {
        status: if db_ok { "ok" } else { "degraded" }.to_string(),
        db_ok,
        claude_auth,
        version: env!("CARGO_PKG_VERSION").to_string(),
        active_slots: (executing + pending_review) as usize,
        max_slots: pipeline_status.max_slots,
        total_slot_capacity: (active_projects as usize).max(1) * pipeline_status.max_slots,
        pending_review: pending_review as usize,
        pause_threshold: pipeline_status.pause_threshold,
        stage,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BadgeCounts {
    pub chat_unread: i64,
    pub audit_pending: i64,
}

/// Lazily check claude CLI auth. Called once from the frontend with a long
/// delay so the subprocess never runs during the critical startup window.
#[tauri::command]
pub async fn check_claude_auth() -> Result<bool, String> {
    Ok(local_claude::check_auth().await)
}

#[tauri::command]
pub async fn get_badge_counts(state: State<'_, AppState>) -> Result<BadgeCounts, String> {
    debug!("[cmd] get_badge_counts start");
    let (chat_unread,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM messages m
         LEFT JOIN conversation_reads r ON r.conversation_id = m.conversation_id
         WHERE m.from_agent IS NOT NULL
           AND m.created_at > COALESCE(r.read_at, '1970-01-01')",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // 功能审计页统一管理两个审核节点：审核 1（issues.pending_review_1）+ 审核 2（change_requests.pending_review_2）
    let (audit_pending,): (i64,) = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM change_requests WHERE status='pending_review_2')
         + (SELECT COUNT(*) FROM issues WHERE status='pending_review_1')",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    debug!(
        "[cmd] get_badge_counts done: chat={} audit={}",
        chat_unread, audit_pending
    );
    Ok(BadgeCounts {
        chat_unread,
        audit_pending,
    })
}

#[tauri::command]
pub async fn pipeline_stats(state: State<'_, AppState>) -> Result<PipelineStats, String> {
    let t0 = std::time::Instant::now();
    debug!("[cmd] pipeline_stats start");
    let concurrency = state.concurrency.status();

    // Batch all issue status counts into a single query.
    let (pending_analysis, pending_review_1, executing_issues, rejected_issues, total_issues): (
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT
           SUM(CASE WHEN status='pending_analysis'  THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='pending_review_1'  THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='executing'          THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='rejected'           THEN 1 ELSE 0 END),
           COUNT(*)
         FROM issues",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Batch all change_request status counts into a single query.
    let (executing_crs, pending_review_2, merged, rejected_crs): (i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT
               SUM(CASE WHEN status='executing'          THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='pending_review_2'   THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='merged'             THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='rejected'           THEN 1 ELSE 0 END)
             FROM change_requests",
        )
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let (active_projects,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status='active'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let executing_cr_ids = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM change_requests WHERE status='executing' ORDER BY updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|(id,)| id)
    .collect::<Vec<_>>();

    let project_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, name, status FROM projects WHERE status='active' ORDER BY name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let occupied_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT cr.project_id, cr.id, cr.status
         FROM change_requests cr
         JOIN projects p ON p.id = cr.project_id
         WHERE p.status='active' AND cr.status IN ('executing', 'pending_review_2')
         ORDER BY cr.updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut occupants_by_project: HashMap<String, Vec<SlotOccupant>> = HashMap::new();
    for (project_id, id, status) in occupied_rows {
        occupants_by_project
            .entry(project_id)
            .or_default()
            .push(SlotOccupant { id, status });
    }

    let issue_pipeline_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, i64)>(
        "SELECT project_id,
                SUM(CASE WHEN status='pending_analysis' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='pending_review_1' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='executing' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END),
                COUNT(*)
         FROM issues
         GROUP BY project_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let cr_pipeline_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT project_id,
                SUM(CASE WHEN status='executing' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='pending_review_2' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='merged' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END)
         FROM change_requests
         GROUP BY project_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let issue_pipeline_by_project = issue_pipeline_rows
        .into_iter()
        .map(
            |(
                project_id,
                pending_analysis,
                pending_review_1,
                executing,
                rejected,
                total_issues,
            )| {
                (
                    project_id,
                    (
                        pending_analysis,
                        pending_review_1,
                        executing,
                        rejected,
                        total_issues,
                    ),
                )
            },
        )
        .collect::<HashMap<_, _>>();

    let cr_pipeline_by_project = cr_pipeline_rows
        .into_iter()
        .map(
            |(project_id, executing, pending_review_2, merged, rejected)| {
                (project_id, (executing, pending_review_2, merged, rejected))
            },
        )
        .collect::<HashMap<_, _>>();

    let project_slots = project_rows
        .iter()
        .map(|(project_id, project_name, project_status)| {
            let occupants = occupants_by_project.remove(project_id).unwrap_or_default();
            let executing_slots = occupants
                .iter()
                .filter(|slot| slot.status == "executing")
                .count();
            let pending_review_slots = occupants
                .iter()
                .filter(|slot| slot.status == "pending_review_2")
                .count();

            ProjectSlotStats {
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                project_status: project_status.clone(),
                active_slots: occupants.len(),
                max_slots: concurrency.max_slots,
                executing_slots,
                pending_review_slots,
                occupants,
            }
        })
        .collect::<Vec<_>>();

    let project_pipelines = project_rows
        .iter()
        .map(|(project_id, project_name, project_status)| {
            let (
                issue_pending_analysis,
                issue_pending_review_1,
                issue_executing,
                issue_rejected,
                project_total_issues,
            ) = issue_pipeline_by_project
                .get(project_id)
                .copied()
                .unwrap_or((0, 0, 0, 0, 0));
            let (cr_executing, cr_pending_review_2, cr_merged, cr_rejected) =
                cr_pipeline_by_project
                    .get(project_id)
                    .copied()
                    .unwrap_or((0, 0, 0, 0));

            ProjectPipelineStats {
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                project_status: project_status.clone(),
                pending_analysis: issue_pending_analysis,
                pending_review_1: issue_pending_review_1,
                executing: issue_executing.max(cr_executing),
                pending_review_2: cr_pending_review_2,
                merged: cr_merged,
                rejected: issue_rejected.max(cr_rejected),
                total_issues: project_total_issues,
            }
        })
        .collect::<Vec<_>>();

    let total_slot_capacity = project_slots.len().max(1) * concurrency.max_slots;
    let active_slots: usize = project_slots
        .iter()
        .map(|project| project.active_slots)
        .sum();
    let pending_review_slots = pending_review_2.max(concurrency.pending_review as i64) as usize;
    let stage = if pending_review_slots >= concurrency.pause_threshold {
        "paused".to_string()
    } else if pending_review_slots >= concurrency.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    let result = Ok(PipelineStats {
        pending_analysis,
        pending_review_1,
        executing: executing_crs.max(executing_issues),
        pending_review_2,
        merged,
        rejected: rejected_crs.max(rejected_issues),
        total_issues,
        active_projects,
        active_slots,
        max_slots: concurrency.max_slots,
        total_slot_capacity,
        pending_review_slots,
        pause_threshold: concurrency.pause_threshold,
        stage,
        executing_cr_ids,
        project_slots,
        project_pipelines,
    });
    info!("[cmd] pipeline_stats done in {:?}", t0.elapsed());
    result
}

#[tauri::command]
pub async fn update_concurrency_config(
    payload: UpdateConcurrencyConfig,
    state: State<'_, AppState>,
) -> Result<ConcurrencyConfig, String> {
    let status = state.concurrency.update_config(
        payload.max_slots,
        payload.pause_threshold,
        payload.queue_strategy,
    );
    let queue_strategy = state.concurrency.queue_strategy();

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.max_slots', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(status.max_slots.to_string())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.pause_threshold', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(status.pause_threshold.to_string())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.queue_strategy', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(&queue_strategy)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(ConcurrencyConfig {
        active_slots: status.active_slots,
        max_slots: status.max_slots,
        pending_review: status.pending_review,
        pause_threshold: status.pause_threshold,
        stage: status.stage,
        queue_strategy,
    })
}

#[tauri::command]
pub async fn get_concurrency_config(
    state: State<'_, AppState>,
) -> Result<ConcurrencyConfig, String> {
    let status = state.concurrency.status();

    Ok(ConcurrencyConfig {
        active_slots: status.active_slots,
        max_slots: status.max_slots,
        pending_review: status.pending_review,
        pause_threshold: status.pause_threshold,
        stage: status.stage,
        queue_strategy: state.concurrency.queue_strategy(),
    })
}

#[tauri::command]
pub async fn list_preview_environments(
    project_id: Option<String>,
    status: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PreviewEnvironment>, String> {
    match (project_id, status) {
        (Some(project_id), Some(status)) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE project_id=? AND status=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .bind(status)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (Some(project_id), None) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (None, Some(status)) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE status=? ORDER BY created_at DESC",
        )
        .bind(status)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (None, None) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments ORDER BY created_at DESC",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn list_test_sessions(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<TestSession>, String> {
    if let Some(project_id) = project_id {
        sqlx::query_as::<_, TestSession>(
            "SELECT * FROM test_sessions WHERE project_id=? ORDER BY COALESCE(started_at, completed_at) DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, TestSession>(
            "SELECT * FROM test_sessions ORDER BY COALESCE(started_at, completed_at) DESC",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn list_scan_findings(
    test_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ScanFinding>, String> {
    if let Some(test_session_id) = test_session_id {
        sqlx::query_as::<_, ScanFinding>(
            "SELECT * FROM scan_findings WHERE test_session_id=? ORDER BY created_at DESC",
        )
        .bind(test_session_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, ScanFinding>("SELECT * FROM scan_findings ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

/// One failed background job, for the Settings 错误历史 panel. Surfaces the
/// `last_error` the runner already persists so failures are inspectable in-app
/// instead of only in stderr logs.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct JobFailure {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub attempt: i64,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Recent failed jobs (most recent first). Capped to avoid unbounded payloads.
#[tauri::command]
pub async fn list_job_failures(
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<JobFailure>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    sqlx::query_as::<_, JobFailure>(
        "SELECT id, job_type, status, attempt, last_error, updated_at
         FROM job_executions WHERE status='failed' ORDER BY updated_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_admin_decisions(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<AdminDecision>, String> {
    if let Some(project_id) = project_id {
        sqlx::query_as::<_, AdminDecision>(
            "SELECT * FROM admin_decisions WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, AdminDecision>("SELECT * FROM admin_decisions ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}
