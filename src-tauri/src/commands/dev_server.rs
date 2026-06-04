use crate::models::project::Project;
use crate::state::{AppState, DevServerHandle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, Deserialize)]
struct ProjectDevConfig {
    dev: Option<DevConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct DevConfig {
    command: String,
    url: String,
}

#[derive(Debug, Serialize)]
pub struct DevServerStatus {
    pub project_id: String,
    // "no_config" | "idle" | "starting" | "running" | "stopped"
    pub status: String,
    pub url: Option<String>,
}

fn parse_dev_config(config_yaml: Option<&str>) -> Option<DevConfig> {
    let raw = config_yaml?;
    let config: ProjectDevConfig = serde_yaml::from_str(raw).ok()?;
    config.dev
}

async fn url_reachable(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .danger_accept_invalid_certs(true)
        .build()
    else {
        return false;
    };
    client.get(url).send().await.is_ok()
}

#[tauri::command]
pub async fn get_dev_server_status(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<DevServerStatus, String> {
    // Clone what we need under the lock to avoid holding it across await
    let handle_info: Option<(Arc<Mutex<Option<tokio::process::Child>>>, String)> = {
        let servers = state.dev_servers.lock().await;
        servers
            .get(&project_id)
            .map(|h| (h.child.clone(), h.url.clone()))
    };

    if let Some((child_arc, url)) = handle_info {
        let child_running = {
            let mut guard = child_arc.lock().await;
            match guard.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        };

        if child_running {
            let reachable = url_reachable(&url).await;
            return Ok(DevServerStatus {
                project_id,
                status: if reachable { "running" } else { "starting" }.to_string(),
                url: Some(url),
            });
        } else {
            state.dev_servers.lock().await.remove(&project_id);
            return Ok(DevServerStatus {
                project_id,
                status: "stopped".to_string(),
                url: Some(url),
            });
        }
    }

    // No running server — check project config to report whether one is available
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let dev = project
        .as_ref()
        .and_then(|p| parse_dev_config(p.config_yaml.as_deref()));

    Ok(DevServerStatus {
        project_id,
        status: if dev.is_some() { "idle" } else { "no_config" }.to_string(),
        url: dev.map(|c| c.url),
    })
}

#[tauri::command]
pub async fn start_dev_server(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<DevServerStatus, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project_id))?;

    let dev = parse_dev_config(project.config_yaml.as_deref())
        .ok_or_else(|| "未配置启动命令 (config_yaml 中缺少 dev.command / dev.url)".to_string())?;

    // Kill any existing process for this project
    {
        let mut servers = state.dev_servers.lock().await;
        if let Some(old) = servers.remove(&project_id) {
            let mut guard = old.child.lock().await;
            if let Some(child) = guard.as_mut() {
                let _ = child.kill().await;
            }
        }
    }

    let child = Command::new("sh")
        .arg("-lc")
        .arg(&dev.command)
        .current_dir(&project.repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("启动失败: {}", e))?;

    let handle = DevServerHandle {
        child: Arc::new(Mutex::new(Some(child))),
        url: dev.url.clone(),
    };

    state
        .dev_servers
        .lock()
        .await
        .insert(project_id.clone(), handle);

    Ok(DevServerStatus {
        project_id,
        status: "starting".to_string(),
        url: Some(dev.url),
    })
}

#[tauri::command]
pub async fn stop_dev_server(project_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut servers = state.dev_servers.lock().await;
    if let Some(handle) = servers.remove(&project_id) {
        let mut guard = handle.child.lock().await;
        if let Some(child) = guard.as_mut() {
            child.kill().await.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
