use crate::models::project::{CreateProject, Project, UpdateProject};
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_project(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Project>, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_project(
    payload: CreateProject,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    let id = Uuid::new_v4().to_string();
    let branch_dev = payload.branch_dev.unwrap_or_else(|| "dev".to_string());
    let branch_main = payload.branch_main.unwrap_or_else(|| "main".to_string());
    let description = payload.description.unwrap_or_default();

    sqlx::query(
        "INSERT INTO projects (id, name, slug, description, repo_path, branch_dev, branch_main) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.slug)
    .bind(&description)
    .bind(&payload.repo_path)
    .bind(&branch_dev)
    .bind(&branch_main)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_project(
    id: String,
    payload: UpdateProject,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    // Build dynamic update
    let mut sets = vec!["updated_at=datetime('now')"];
    let mut values: Vec<String> = vec![];

    if let Some(ref v) = payload.name {
        sets.push("name=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.description {
        sets.push("description=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.repo_path {
        sets.push("repo_path=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.branch_dev {
        sets.push("branch_dev=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.branch_main {
        sets.push("branch_main=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.status {
        sets.push("status=?");
        values.push(v.clone());
    }
    if let Some(ref v) = payload.config_yaml {
        sets.push("config_yaml=?");
        values.push(v.clone());
    }

    let sql = format!("UPDATE projects SET {} WHERE id=?", sets.join(", "));
    let mut q = sqlx::query(&sql);
    for v in &values {
        q = q.bind(v);
    }
    q.bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM projects WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
