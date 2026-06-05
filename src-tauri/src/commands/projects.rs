use crate::models::project::{CloneProject, CreateProject, Project, UpdateProject};
use crate::state::AppState;
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_active_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE status = 'active' ORDER BY created_at DESC",
    )
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
        "INSERT INTO projects (id, name, slug, description, repo_path, branch_dev, branch_main, config_yaml) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&payload.name)
    .bind(&payload.slug)
    .bind(&description)
    .bind(&payload.repo_path)
    .bind(&branch_dev)
    .bind(&branch_main)
    .bind(&payload.config_yaml)
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
pub async fn create_local_project(
    payload: CreateProject,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    ensure_local_project_dir(&payload.repo_path, payload.branch_main.as_deref()).await?;
    create_project(payload, state).await
}

#[tauri::command]
pub async fn clone_project_from_git(
    payload: CloneProject,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    let git_url = payload.git_url.trim();
    if git_url.is_empty() {
        return Err("Git 地址不能为空".into());
    }
    let target = PathBuf::from(payload.target_path.trim());
    if target.as_os_str().is_empty() {
        return Err("本地目录不能为空".into());
    }
    if target.exists() {
        let mut entries = tokio::fs::read_dir(&target)
            .await
            .map_err(|e| e.to_string())?;
        if entries
            .next_entry()
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("目标目录已存在且不是空目录".into());
        }
    } else if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    let username = payload.git_username.as_deref().map(str::trim).unwrap_or("");
    let password = payload.git_password.as_deref().unwrap_or("");
    if username.is_empty() != password.is_empty() {
        return Err("Git 认证需要同时填写用户名和密码/Token".into());
    }

    let askpass_path = write_git_askpass(username, password).await?;
    let mut cmd = Command::new("git");
    cmd.arg("-c").arg("credential.helper=").arg("clone");
    if let Some(branch) = payload
        .clone_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cmd.arg("--branch").arg(branch);
    }
    cmd.arg(git_url).arg(&target);
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", &askpass_path)
        .env("SSH_ASKPASS", &askpass_path)
        .env("GCM_INTERACTIVE", "never")
        .env("AUTOFORGE_GIT_USERNAME", username)
        .env("AUTOFORGE_GIT_PASSWORD", password);
    let output = match timeout(Duration::from_secs(600), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&askpass_path).await;
            return Err(format!("无法执行 git clone: {}", e));
        }
        Err(_) => {
            let _ = tokio::fs::remove_file(&askpass_path).await;
            return Err("git clone 超时".into());
        }
    };
    let _ = tokio::fs::remove_file(&askpass_path).await;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not read Username")
            || stderr.contains("could not read Password")
            || stderr.contains("Authentication failed")
            || stderr.contains("No such device or address")
            || stderr.contains("terminal prompts disabled")
        {
            return Err(
                "git clone 失败：该仓库需要认证，请填写 Git 用户名和密码/Token 后重试".into(),
            );
        }
        return Err(format!("git clone 失败：{}", stderr.trim()));
    }

    let project_payload = CreateProject {
        name: payload.name,
        slug: payload.slug,
        description: payload.description,
        repo_path: target.to_string_lossy().to_string(),
        branch_dev: payload.branch_dev,
        branch_main: payload.branch_main,
        config_yaml: payload.config_yaml,
    };
    create_project(project_payload, state).await
}

async fn write_git_askpass(username: &str, password: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("autoforge-git-askpass-{}.sh", Uuid::new_v4()));
    let script = if username.is_empty() && password.is_empty() {
        "#!/bin/sh\nexit 1\n".to_string()
    } else {
        r#"#!/bin/sh
case "$1" in
  *Username*|*username*) printf '%s\n' "$AUTOFORGE_GIT_USERNAME" ;;
  *) printf '%s\n' "$AUTOFORGE_GIT_PASSWORD" ;;
esac
"#
        .to_string()
    };
    tokio::fs::write(&path, script)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&path)
            .await
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o700);
        tokio::fs::set_permissions(&path, perms)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
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

async fn ensure_local_project_dir(
    repo_path: &str,
    branch_main: Option<&str>,
) -> Result<(), String> {
    let path = PathBuf::from(repo_path.trim());
    if path.as_os_str().is_empty() {
        return Err("本地目录不能为空".into());
    }

    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| e.to_string())?;
    if path.join(".git").exists() {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("init");
    if let Some(branch) = branch_main.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.arg("--initial-branch").arg(branch);
    }
    cmd.current_dir(Path::new(&path));
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("无法执行 git init: {}", e))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("unknown option") || stderr.contains("usage: git init") {
        let fallback = Command::new("git")
            .arg("init")
            .current_dir(Path::new(&path))
            .output()
            .await
            .map_err(|e| format!("无法执行 git init: {}", e))?;
        if fallback.status.success() {
            return Ok(());
        }
        return Err(format!(
            "git init 失败：{}",
            String::from_utf8_lossy(&fallback.stderr).trim()
        ));
    }

    Err(format!("git init 失败：{}", stderr.trim()))
}

#[tauri::command]
pub async fn delete_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("project {} not found", id));
    }

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM scan_findings
         WHERE test_session_id IN (SELECT id FROM test_sessions WHERE project_id=?)
            OR issue_entry_id IN (SELECT id FROM issues WHERE project_id=?)",
    )
    .bind(&id)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM admin_decisions WHERE project_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM preview_environments WHERE project_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM test_sessions WHERE project_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM worktree_sessions
         WHERE change_request_id IN (SELECT id FROM change_requests WHERE project_id=?)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM change_requests WHERE project_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM issue_analyses
         WHERE issue_id IN (SELECT id FROM issues WHERE project_id=?)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM issues WHERE project_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM projects WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())
}
