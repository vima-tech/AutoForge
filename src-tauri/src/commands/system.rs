use crate::agents::local_claude;
use crate::models::{
    admin_decision::AdminDecision,
    preview::PreviewEnvironment,
    test_session::{ScanFinding, TestSession},
};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemHealth {
    pub status: String,
    pub db_ok: bool,
    pub claude_auth: bool,
    pub version: String,
    pub active_slots: usize,
    pub max_slots: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
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
    pub pending_review_slots: usize,
    pub pause_threshold: usize,
    pub stage: String,
    pub executing_cr_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecDocument {
    pub name: String,
    pub content: String,
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

#[tauri::command]
pub async fn system_health(state: State<'_, AppState>) -> Result<SystemHealth, String> {
    // Check DB
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    // Check claude auth
    let claude_auth = local_claude::check_auth().await;

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
    let stage = if pending_review as usize >= pipeline_status.pause_threshold {
        "paused".to_string()
    } else if pending_review as usize >= pipeline_status.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    Ok(SystemHealth {
        status: if db_ok { "ok" } else { "degraded" }.to_string(),
        db_ok,
        claude_auth,
        version: env!("CARGO_PKG_VERSION").to_string(),
        active_slots: (executing + pending_review) as usize,
        max_slots: pipeline_status.max_slots,
        pending_review: pending_review as usize,
        pause_threshold: pipeline_status.pause_threshold,
        stage,
    })
}

#[tauri::command]
pub async fn pipeline_stats(state: State<'_, AppState>) -> Result<PipelineStats, String> {
    let concurrency = state.concurrency.status();

    let (pending_analysis,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status='pending_analysis'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (pending_review_1,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status='pending_review_1'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (executing_issues,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status='executing'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (executing_crs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='executing'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (pending_review_2,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='pending_review_2'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (merged,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='merged'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (rejected_issues,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM issues WHERE status='rejected'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (rejected_crs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='rejected'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let (total_issues,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let (active_projects,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status='active'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let executing_cr_ids = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM change_requests WHERE status IN ('executing', 'pending_review_2') ORDER BY updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|(id,)| id)
    .collect::<Vec<_>>();

    let active_slots = executing_cr_ids.len().max(concurrency.active_slots);
    let pending_review_slots = pending_review_2.max(concurrency.pending_review as i64) as usize;
    let stage = if pending_review_slots >= concurrency.pause_threshold {
        "paused".to_string()
    } else if pending_review_slots >= concurrency.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    Ok(PipelineStats {
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
        pending_review_slots,
        pause_threshold: concurrency.pause_threshold,
        stage,
        executing_cr_ids,
    })
}

#[tauri::command]
pub async fn read_spec(name: String) -> Result<SpecDocument, String> {
    let path = spec_path(&name)?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("读取规范失败: {}", e))?;

    Ok(SpecDocument {
        name: path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(&name)
            .to_string(),
        content,
    })
}

#[tauri::command]
pub async fn write_spec(name: String, content: String) -> Result<SpecDocument, String> {
    let path = spec_path(&name)?;
    tokio::fs::write(&path, content.as_bytes())
        .await
        .map_err(|e| format!("写入规范失败: {}", e))?;

    Ok(SpecDocument {
        name: path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(&name)
            .to_string(),
        content,
    })
}

fn spec_path(name: &str) -> Result<PathBuf, String> {
    let allowed = [
        "analysis-spec.md",
        "coding-spec.md",
        "review-spec.md",
        "testing-spec.md",
    ];
    if !allowed.contains(&name) {
        return Err("未知规范文档".to_string());
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let repo_root = if cwd.file_name().and_then(|v| v.to_str()) == Some("src-tauri") {
        cwd.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        cwd
    };

    Ok(repo_root.join("specs").join(name))
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
