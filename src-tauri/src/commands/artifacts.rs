//! Delivery node artifacts — unified management of a project's delivery materials.
//! Files are persisted under the project repo's `.autoforge/deliverables/<node>/`
//! so they live alongside the rest of the project's AutoForge workspace.

use crate::models::artifact::DeliveryArtifact;
use crate::state::AppState;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::State;
use uuid::Uuid;

const DELIVERABLES_DIR: &str = ".autoforge/deliverables";

async fn repo_path(db: &crate::db::Db, project_id: &str) -> Result<String, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?;
    let (p,) = row.ok_or("项目不存在")?;
    if p.is_empty() {
        return Err("项目未设置仓库路径".to_string());
    }
    Ok(p)
}

fn safe(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed
    }
}

#[tauri::command]
pub async fn list_delivery_artifacts(
    project_id: String,
    node: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<DeliveryArtifact>, String> {
    match node {
        Some(n) => sqlx::query_as::<_, DeliveryArtifact>(
            "SELECT * FROM delivery_artifacts WHERE project_id=? AND node=? ORDER BY created_at DESC",
        )
        .bind(&project_id)
        .bind(&n)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        None => sqlx::query_as::<_, DeliveryArtifact>(
            "SELECT * FROM delivery_artifacts WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(&project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn import_delivery_artifact(
    project_id: String,
    node: String,
    file_name: String,
    mime: String,
    data_base64: String,
    state: State<'_, AppState>,
) -> Result<DeliveryArtifact, String> {
    let repo = repo_path(&state.db, &project_id).await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("base64 解码失败: {e}"))?;

    let sha256 = hex::encode(Sha256::digest(&bytes));
    let node_safe = safe(&node);
    let stored = format!("{}_{}", &Uuid::new_v4().to_string()[..8], safe(&file_name));
    let rel_path = format!("{DELIVERABLES_DIR}/{node_safe}/{stored}");

    let dir = PathBuf::from(&repo).join(DELIVERABLES_DIR).join(&node_safe);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(dir.join(&stored), &bytes)
        .await
        .map_err(|e| e.to_string())?;

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO delivery_artifacts
         (id, project_id, node, original_name, stored_name, rel_path, mime, size_bytes, sha256)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&node_safe)
    .bind(&file_name)
    .bind(&stored)
    .bind(&rel_path)
    .bind(&mime)
    .bind(bytes.len() as i64)
    .bind(&sha256)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, DeliveryArtifact>("SELECT * FROM delivery_artifacts WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_delivery_artifact_meta(
    id: String,
    description: String,
    state: State<'_, AppState>,
) -> Result<DeliveryArtifact, String> {
    sqlx::query("UPDATE delivery_artifacts SET description=? WHERE id=?")
        .bind(&description)
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, DeliveryArtifact>("SELECT * FROM delivery_artifacts WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_delivery_artifact(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT project_id, rel_path FROM delivery_artifacts WHERE id=?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    if let Some((project_id, rel_path)) = row {
        if let Ok(repo) = repo_path(&state.db, &project_id).await {
            let _ = tokio::fs::remove_file(PathBuf::from(&repo).join(&rel_path)).await;
        }
    }
    sqlx::query("DELETE FROM delivery_artifacts WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Return a base64 data URL for download/preview in the UI.
#[tauri::command]
pub async fn delivery_artifact_data_url(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT project_id, rel_path, mime FROM delivery_artifacts WHERE id=?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (project_id, rel_path, mime) = row.ok_or("产物不存在")?;
    let repo = repo_path(&state.db, &project_id).await?;
    let bytes = tokio::fs::read(PathBuf::from(&repo).join(&rel_path))
        .await
        .map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mime = if mime.is_empty() {
        "application/octet-stream"
    } else {
        &mime
    };
    Ok(format!("data:{mime};base64,{b64}"))
}
