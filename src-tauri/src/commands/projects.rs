use crate::models::project::{CloneProject, CreateProject, Project, UpdateProject};
use crate::state::AppState;
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

/// 用文件真源覆盖 config_yaml，使前端/派生逻辑看到的是 .autoforge/run-config.json 的内容。
fn overlay_config(mut p: Project) -> Project {
    p.config_yaml = crate::commands::run_config::effective_config(&p);
    p
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE archived_at IS NULL ORDER BY is_default DESC, created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(projects.into_iter().map(overlay_config).collect())
}

/// 回收站：列出已归档（软删除）的项目。
#[tauri::command]
pub async fn list_archived_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE archived_at IS NOT NULL ORDER BY archived_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(projects.into_iter().map(overlay_config).collect())
}

#[tauri::command]
pub async fn list_active_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE status = 'active' AND archived_at IS NULL ORDER BY is_default DESC, created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(projects.into_iter().map(overlay_config).collect())
}

#[tauri::command]
pub async fn get_project(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Project>, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(project.map(overlay_config))
}

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// 抗重复项目 id：毫秒时间戳前缀 + UUIDv4。
/// 在 UUID 本就极低的碰撞率上再叠加单调时间维度，最大限度避免重复。
fn new_project_id() -> String {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    format!("{}-{}", ts, Uuid::new_v4())
}

/// 仓库内身份锚文件：`<repo>/.autoforge/project.json`。
#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectIdentity {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    created_at: String,
}

fn identity_file_path(repo_path: &str) -> Option<PathBuf> {
    let repo = PathBuf::from(repo_path.trim());
    if repo.as_os_str().is_empty() {
        return None;
    }
    Some(repo.join(".autoforge").join("project.json"))
}

/// id 安全校验：非空、长度合理、仅安全字符（兼容旧纯 UUID 与新「时间戳-UUID」复合格式）。
fn is_valid_project_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// 读取仓库内身份锚里的合法项目 id；不存在/损坏/非法一律视为无身份。
fn read_identity(repo_path: &str) -> Option<String> {
    let path = identity_file_path(repo_path)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let ident: ProjectIdentity = serde_json::from_str(&text).ok()?;
    let id = ident.id.trim().to_string();
    if is_valid_project_id(&id) {
        Some(id)
    } else {
        None
    }
}

/// 写入/刷新仓库内身份锚（容错：路径无效或写失败仅静默，不阻断建项目）。
fn write_identity(repo_path: &str, id: &str, name: &str) {
    let Some(path) = identity_file_path(repo_path) else { return; };
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let ident = ProjectIdentity {
        id: id.to_string(),
        name: name.to_string(),
        created_at: now_str(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&ident) {
        let _ = std::fs::write(&path, text);
    }
}

/// 启动时一次性补全：为已有项目写入仓库内身份锚 `.autoforge/project.json`（若缺失）。
/// 幂等且非破坏——仅当仓库目录存在、且尚无合法身份文件时写入；已有锚不覆盖、
/// 仓库路径为空或不存在则跳过。不触碰任何 DB 数据，故不影响现有项目的正常使用。
pub async fn backfill_project_identities(db: &sqlx::SqlitePool) {
    let rows = match sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, name, repo_path FROM projects WHERE repo_path IS NOT NULL AND repo_path != ''",
    )
    .fetch_all(db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[identity] 身份锚补全查询失败: {}", e);
            return;
        }
    };
    let mut written = 0usize;
    for (id, name, repo_path) in rows {
        if !PathBuf::from(repo_path.trim()).is_dir() {
            continue; // 仓库目录不存在 → 不创建幽灵目录
        }
        if read_identity(&repo_path).is_some() {
            continue; // 已有合法身份锚 → 不覆盖
        }
        write_identity(&repo_path, &id, &name);
        written += 1;
    }
    if written > 0 {
        tracing::info!("[identity] 已为 {} 个已有项目补全 .autoforge/project.json 身份锚", written);
    }
}

async fn insert_project_row(
    id: &str,
    payload: &CreateProject,
    branch_dev: &str,
    branch_main: &str,
    description: &str,
    state: &AppState,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO projects (id, name, slug, description, repo_path, branch_dev, branch_main, config_yaml) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(id)
    .bind(&payload.name)
    .bind(&payload.slug)
    .bind(description)
    .bind(&payload.repo_path)
    .bind(branch_dev)
    .bind(branch_main)
    .bind(&payload.config_yaml)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn fetch_project(id: &str, state: &AppState) -> Result<Project, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_project(
    payload: CreateProject,
    state: State<'_, AppState>,
) -> Result<Project, String> {
    let branch_dev = payload.branch_dev.clone().unwrap_or_else(|| "dev".to_string());
    let branch_main = payload.branch_main.clone().unwrap_or_else(|| "main".to_string());
    let description = payload.description.clone().unwrap_or_default();
    let app = state.inner();

    // 1) 仓库内身份锚优先：读到合法 id 就尝试挂回历史数据（含已归档项目）。
    if let Some(fid) = read_identity(&payload.repo_path) {
        let existing = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
            .bind(&fid)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        match existing {
            Some(p) => {
                let same_repo = p.repo_path.trim().is_empty()
                    || p.repo_path.trim() == payload.repo_path.trim();
                let archived = p.archived_at.is_some();
                // 同仓库或已归档 → 重新挂回（清归档、刷新基本信息，不动子表数据）。
                // 否则该 id 被另一在用且不同仓库的项目占用 = 复制仓库，落到下方新建独立身份。
                if same_repo || archived {
                    let now = now_str();
                    sqlx::query(
                        "UPDATE projects SET name=?, slug=?, description=?, repo_path=?, branch_dev=?, branch_main=?, config_yaml=?, status='active', archived_at=NULL, updated_at=? WHERE id=?",
                    )
                    .bind(&payload.name)
                    .bind(&payload.slug)
                    .bind(&description)
                    .bind(&payload.repo_path)
                    .bind(&branch_dev)
                    .bind(&branch_main)
                    .bind(&payload.config_yaml)
                    .bind(&now)
                    .bind(&fid)
                    .execute(&state.db)
                    .await
                    .map_err(|e| e.to_string())?;
                    write_identity(&payload.repo_path, &fid, &payload.name);
                    let _ = crate::commands::specs::reconcile_specs_from_disk(&fid, app).await;
                    return fetch_project(&fid, app).await;
                }
            }
            None => {
                // DB 无此 id（换机/清库）→ 沿用文件 id 新建，保持身份一致后从磁盘恢复规格。
                insert_project_row(&fid, &payload, &branch_dev, &branch_main, &description, app).await?;
                write_identity(&payload.repo_path, &fid, &payload.name);
                let _ = crate::commands::specs::reconcile_specs_from_disk(&fid, app).await;
                return fetch_project(&fid, app).await;
            }
        }
    }

    // 2) 默认 / 复制碰撞：生成抗重复新 id（时间戳+UUID），写身份锚。
    let id = new_project_id();
    insert_project_row(&id, &payload, &branch_dev, &branch_main, &description, app).await?;
    write_identity(&payload.repo_path, &id, &payload.name);
    // 仓库可能已带 .autoforge/specs（如 clone 的模板）→ 一并对账登记。
    let _ = crate::commands::specs::reconcile_specs_from_disk(&id, app).await;
    fetch_project(&id, app).await
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
        // Restrict transports to a safe whitelist so a hostile URL cannot use
        // remote helpers like `ext::sh -c ...` to achieve command execution.
        .env("GIT_ALLOW_PROTOCOL", "http:https:ssh:git:file")
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

    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // 运行配置真源是 .autoforge/run-config.json：保存时落盘（YAML/JSON→规整 JSON）。
    if let Some(ref cfg) = payload.config_yaml {
        crate::commands::run_config::write_config_file(&project.repo_path, cfg)?;
    }
    Ok(overlay_config(project))
}

/// 设置默认项目：全表唯一。其他页面按 `is_default DESC` 排序后将其置顶并优先选中。
/// `id` 为空字符串时表示清除默认项目。
#[tauri::command]
pub async fn set_default_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE projects SET is_default = 0 WHERE is_default = 1")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if !id.is_empty() {
        let res = sqlx::query("UPDATE projects SET is_default = 1 WHERE id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if res.rows_affected() == 0 {
            return Err(format!("project {} not found", id));
        }
    }
    tx.commit().await.map_err(|e| e.to_string())
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

/// 软删除：项目归档（保留全部 DB 数据），配合 `.autoforge/project.json` 身份锚，
/// 重新添加同一仓库时自动挂回。真正彻底清除走 `purge_project`。
#[tauri::command]
pub async fn delete_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let now = now_str();
    let res = sqlx::query(
        "UPDATE projects SET archived_at=?, status='inactive', updated_at=? WHERE id=? AND archived_at IS NULL",
    )
    .bind(&now)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("project {} not found", id));
        }
        // 已归档：幂等返回成功。
    }
    Ok(())
}

/// 从回收站恢复一个已归档项目。
#[tauri::command]
pub async fn restore_project(id: String, state: State<'_, AppState>) -> Result<Project, String> {
    let now = now_str();
    let res = sqlx::query(
        "UPDATE projects SET archived_at=NULL, status='active', updated_at=? WHERE id=?",
    )
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err(format!("project {} not found", id));
    }
    fetch_project(&id, state.inner()).await.map(overlay_config)
}

/// 彻底删除（不可恢复）：级联清除该项目所有 DB 数据。仓库内 `.autoforge/` 文件不动。
#[tauri::command]
pub async fn purge_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
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
