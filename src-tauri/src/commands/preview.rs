use crate::models::change_request::ChangeRequest;
use crate::models::project::Project;
use crate::models::worktree::WorktreeSession;
use crate::state::AppState;
use tauri::State;

/// Manually (re)apply M5 field masking to a change request's preview database.
#[tauri::command]
pub async fn mask_preview_data(cr_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("cr {cr_id} not found"))?;
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&cr.project_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let session = sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "无 worktree 会话".to_string())?;
    let preview: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM preview_environments WHERE worktree_session_id=? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&session.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some((preview_id,)) = preview else {
        return Err("无预览环境".to_string());
    };
    crate::tasks::preview::apply_preview_masking(&state.db, &project, &preview_id, &session.worktree_path).await;
    Ok(())
}

/// M5 — build + run a container preview for a change request (Podman/Docker).
/// Requires a Dockerfile in the worktree + a container runtime; returns the URL.
#[tauri::command]
pub async fn provision_preview_container(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("cr {cr_id} not found"))?;
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&cr.project_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let session = sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&cr_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "无 worktree 会话".to_string())?;
    let preview: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM preview_environments WHERE worktree_session_id=? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&session.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some((preview_id,)) = preview else {
        return Err("无预览环境".to_string());
    };
    crate::tasks::preview::provision_container(&state.db, &project, &preview_id, &session.worktree_path).await
}
