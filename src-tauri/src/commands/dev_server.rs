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
}

/// Preview always loads the dev server at localhost on the auto-allocated port;
/// the URL is no longer project-configurable (`{port}` is substituted per-run).
const DEFAULT_DEV_URL: &str = "http://localhost:{port}";

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

/// Deterministic per-project starting port in [19000, 23000) — a stable hint so the
/// same project tends to reuse the same port, while `free_port_from` guarantees the
/// one actually used is free. Crucial when the target project pins Vite to a fixed
/// port (e.g. Tauri's 1420) that clashes with an already-running instance.
fn derive_port(seed: &str) -> u16 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    19000 + (h % 4000) as u16
}

/// Probe for a free TCP port starting at `base`, scanning upward, then falling back
/// to an OS-assigned ephemeral port. The brief bind is dropped immediately so the
/// dev server can claim it (a tiny TOCTOU window, acceptable for local dev).
pub(crate) fn free_port_from(base: u16) -> u16 {
    use std::net::TcpListener;
    for p in base..base.saturating_add(400) {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(base)
}

fn apply_port(template: &str, port: u16) -> String {
    template.replace("{port}", &port.to_string())
}

/// Make a dev command bind a specific port automatically.
/// - `{port}` placeholder → substituted.
/// - already specifies `--port` / `-p` → respected as-is (user opted out).
/// - npm/pnpm/bun/yarn/vite scripts → append the right `--port` flag.
/// - anything else → unchanged (the child still gets `PORT` via env).
pub(crate) fn inject_port(command: &str, port: u16) -> String {
    if command.contains("{port}") {
        return apply_port(command, port);
    }
    if command.contains("--port") || command.contains(" -p ") {
        return command.to_string();
    }
    // `tauri dev` has no --port flag — it controls the frontend port via tauri.conf.
    // Injecting one would crash it; leave such commands untouched.
    if command.contains("tauri") {
        return command.to_string();
    }
    let t = command.trim_start();
    if t.starts_with("npm ") || t.starts_with("pnpm ") || t.starts_with("bun ") {
        format!("{command} -- --port {port}")
    } else if t.starts_with("yarn ") || t.starts_with("vite") || t.starts_with("npx ") {
        format!("{command} --port {port}")
    } else {
        command.to_string()
    }
}

/// Whether a literal command string references the `PORT` env var (`$PORT` / `${PORT}`).
pub(crate) fn command_uses_port_env(command: &str) -> bool {
    command.contains("$PORT") || command.contains("${PORT")
}

/// Extract the npm-style script name a command runs, if any:
/// `npm run X`, `pnpm run X`, `pnpm X`, `yarn X`, `bun run X` → `Some("X")`.
/// Returns `None` for direct binaries (`vite`, `python …`, `tauri dev`, …).
fn npm_script_name(command: &str) -> Option<String> {
    let mut toks = command.split_whitespace();
    let runner = toks.next()?;
    match runner {
        "npm" | "bun" => {
            // require an explicit `run`
            if toks.next()? != "run" {
                return None;
            }
            toks.next().map(|s| s.to_string())
        }
        "pnpm" | "yarn" => {
            let next = toks.next()?;
            if next == "run" {
                toks.next().map(|s| s.to_string())
            } else {
                Some(next.to_string())
            }
        }
        _ => None,
    }
}

/// Whether the command (resolved through its package.json script body, if it's an
/// npm-style script invocation) consumes the `PORT` env var we export. Such commands
/// — e.g. a `tauri:dev` script whose frontend port is `${PORT:-1420}` — run on our
/// allocated port even though `inject_port` leaves their text untouched, so the
/// configured URL must be realigned to match. `repo_path` resolves the script body.
pub(crate) fn command_consumes_port(command: &str, repo_path: &str) -> bool {
    if command_uses_port_env(command) {
        return true;
    }
    let Some(script) = npm_script_name(command) else {
        return false;
    };
    let pkg = std::path::Path::new(repo_path).join("package.json");
    let Ok(raw) = std::fs::read_to_string(&pkg) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    json.get("scripts")
        .and_then(|s| s.get(&script))
        .and_then(|v| v.as_str())
        .map(command_uses_port_env)
        .unwrap_or(false)
}

/// Resolve the URL to poll, aligned to the actually-used `port`.
/// `port_applied` is true when the server actually listens on `port` — either
/// because we injected a `--port` flag or because the command reads `$PORT`.
fn resolve_url(template: &str, port_applied: bool, port: u16) -> String {
    if template.contains("{port}") {
        apply_port(template, port)
    } else if template.trim().is_empty() || port_applied {
        // the running server uses our allocated port → the configured URL's
        // hardcoded port (e.g. 1420) no longer applies
        format!("http://localhost:{port}")
    } else {
        template.to_string()
    }
}

async fn url_reachable(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
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
        .and_then(|p| parse_dev_config(crate::commands::run_config::effective_config(p).as_deref()));

    let base = derive_port(&project_id);
    Ok(DevServerStatus {
        project_id,
        status: if dev.is_some() { "idle" } else { "no_config" }.to_string(),
        url: dev.map(|_| apply_port(DEFAULT_DEV_URL, base)),
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

    let dev = parse_dev_config(crate::commands::run_config::effective_config(&project).as_deref())
        .ok_or_else(|| "未配置启动命令 (config_yaml 中缺少 dev.command)".to_string())?;

    // Auto-allocate a free port so the preview never collides with an already-running
    // instance (e.g. the target project pins Vite to 1420). The port is injected into
    // the command (and exported as PORT) and the URL is aligned to it.
    let port = free_port_from(derive_port(&project_id));
    let command = inject_port(&dev.command, port);
    // The URL must follow the port when we either rewrote the command or the
    // command (directly or via its package.json script) reads the PORT env we
    // export (e.g. a `tauri:dev` script whose devUrl is `${PORT:-1420}`).
    let port_applied =
        command != dev.command || command_consumes_port(&dev.command, &project.repo_path);
    let url = resolve_url(DEFAULT_DEV_URL, port_applied, port);

    // Kill any existing process for this project
    {
        let mut servers = state.dev_servers.lock().await;
        if let Some(old) = servers.remove(&project_id) {
            let mut guard = old.child.lock().await;
            if let Some(child) = guard.as_mut() {
                kill_child_group(child).await;
            }
        }
    }

    // Capture stdout+stderr to a per-project log file so启动失败 is diagnosable
    // (previously piped to /dev/null, which made every failure invisible).
    let log_path = dev_log_path(&project_id);
    let mut log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("无法创建日志文件: {}", e))?;
    {
        use std::io::Write;
        let _ = writeln!(
            log_file,
            "$ {}\n# cwd: {}\n# port={} url={}\n# PATH={}\n----------------------------------------",
            command,
            project.repo_path,
            port,
            url,
            std::env::var("PATH").unwrap_or_default()
        );
    }
    let log_err = log_file
        .try_clone()
        .map_err(|e| format!("日志文件克隆失败: {}", e))?;

    let mut cmd = crate::core::platform::shell(&command);
    cmd.current_dir(&project.repo_path)
        .env("PORT", port.to_string())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err));
    // Run in its own process group so the whole tree (shell → npm → node …)
    // can be torn down together; killing only the shell would orphan the rest.
    crate::core::platform::detach_process_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    let handle = DevServerHandle {
        child: Arc::new(Mutex::new(Some(child))),
        url: url.clone(),
    };

    state
        .dev_servers
        .lock()
        .await
        .insert(project_id.clone(), handle);

    Ok(DevServerStatus {
        project_id,
        status: "starting".to_string(),
        url: Some(url),
    })
}

fn dev_log_path(project_id: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("autoforge-dev-{}.log", project_id))
}

/// Return the tail of the dev server's captured log (stdout+stderr). Empty string
/// when no run has happened yet. Used by the审计 page to show why启动 failed.
#[tauri::command]
pub async fn get_dev_server_log(project_id: String) -> Result<String, String> {
    let path = dev_log_path(&project_id);
    match std::fs::read_to_string(&path) {
        Ok(s) if s.len() > 16000 => Ok(s[s.len() - 16000..].to_string()),
        Ok(s) => Ok(s),
        Err(_) => Ok(String::new()),
    }
}

#[tauri::command]
pub async fn stop_dev_server(project_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut servers = state.dev_servers.lock().await;
    if let Some(handle) = servers.remove(&project_id) {
        let mut guard = handle.child.lock().await;
        if let Some(child) = guard.as_mut() {
            kill_child_group(child).await;
        }
    }
    Ok(())
}

/// Kill an entire dev-server process tree. The child is spawned in its own
/// process group (unix: `setpgid`; windows: `CREATE_NEW_PROCESS_GROUP`), so the
/// whole tree (shell → npm → node, …) can be reaped together instead of orphaning
/// grandchildren. Falls back to killing the direct child for reaping.
pub(crate) async fn kill_child_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        // PID == process-group id; signal the negative PID to reap the whole group.
        if let Some(pid) = child.id() {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(format!("-{}", pid))
                .output()
                .await;
        }
    }
    #[cfg(windows)]
    {
        // TerminateProcess on cmd.exe alone leaves npm/node orphaned; taskkill
        // with /T walks and kills the child tree by PID.
        if let Some(pid) = child.id() {
            let _ = Command::new("taskkill")
                .arg("/F")
                .arg("/T")
                .arg("/PID")
                .arg(pid.to_string())
                .output()
                .await;
        }
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_substitutes_placeholder() {
        assert_eq!(inject_port("npm run dev -- --port {port}", 5200), "npm run dev -- --port 5200");
        assert_eq!(inject_port("http://localhost:{port}", 5200), "http://localhost:5200");
    }

    #[test]
    fn inject_appends_for_known_runners() {
        assert_eq!(inject_port("npm run dev", 5200), "npm run dev -- --port 5200");
        assert_eq!(inject_port("pnpm dev", 5200), "pnpm dev -- --port 5200");
        assert_eq!(inject_port("yarn dev", 5200), "yarn dev --port 5200");
        assert_eq!(inject_port("vite", 5200), "vite --port 5200");
    }

    #[test]
    fn inject_respects_explicit_port_and_unknown_commands() {
        assert_eq!(inject_port("npm run dev -- --port 3000", 5200), "npm run dev -- --port 3000");
        assert_eq!(inject_port("python -m http.server", 5200), "python -m http.server");
        // tauri dev manages its own port — must stay untouched
        assert_eq!(inject_port("npm run tauri:dev", 5200), "npm run tauri:dev");
    }

    #[test]
    fn resolve_url_aligns_to_port() {
        assert_eq!(resolve_url("http://localhost:{port}", false, 5200), "http://localhost:5200");
        // command was auto-injected → ignore the stale configured port
        assert_eq!(resolve_url("http://localhost:1420", true, 5200), "http://localhost:5200");
        // user kept an explicit URL and we didn't touch the command → respect it
        assert_eq!(resolve_url("http://localhost:3000", false, 5200), "http://localhost:3000");
        assert_eq!(resolve_url("", false, 5200), "http://localhost:5200");
    }

    #[test]
    fn port_env_commands_realign_url() {
        // A `tauri dev` script whose frontend port is `${PORT:-1420}`: text is left
        // untouched by inject_port, but it runs on our allocated port, so the URL must
        // be realigned instead of showing the stale 1420.
        let raw = "tauri dev --config '{\"build\":{\"devUrl\":\"http://localhost:${PORT:-1420}\"}}'";
        assert!(command_uses_port_env(raw));
        assert!(!command_uses_port_env("npm run tauri:dev"));
        // when the command consumes PORT, the stale configured URL is replaced
        assert_eq!(resolve_url("http://127.0.0.1:1420", true, 22164), "http://localhost:22164");
    }

    #[test]
    fn parses_npm_style_script_names() {
        assert_eq!(npm_script_name("npm run tauri:dev").as_deref(), Some("tauri:dev"));
        assert_eq!(npm_script_name("bun run dev").as_deref(), Some("dev"));
        assert_eq!(npm_script_name("pnpm run dev").as_deref(), Some("dev"));
        assert_eq!(npm_script_name("pnpm dev").as_deref(), Some("dev"));
        assert_eq!(npm_script_name("yarn dev").as_deref(), Some("dev"));
        assert_eq!(npm_script_name("yarn run dev").as_deref(), Some("dev"));
        assert_eq!(npm_script_name("vite"), None);
        assert_eq!(npm_script_name("python -m http.server"), None);
        assert_eq!(npm_script_name("npm install"), None);
    }

    #[test]
    fn resolves_port_env_through_package_json() {
        let dir = std::env::temp_dir().join(format!("autoforge-devsrv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pkg = dir.join("package.json");
        std::fs::write(
            &pkg,
            r#"{"scripts":{"tauri:dev":"tauri dev --config \"{\\\"build\\\":{\\\"devUrl\\\":\\\"http://localhost:${PORT:-1420}\\\"}}\"","dev":"vite"}}"#,
        )
        .unwrap();
        let repo = dir.to_str().unwrap();
        // script body references ${PORT} → port is applied
        assert!(command_consumes_port("npm run tauri:dev", repo));
        // plain vite script does not → URL stays as configured
        assert!(!command_consumes_port("npm run dev", repo));
        // missing package.json / unknown script → false, not a panic
        assert!(!command_consumes_port("npm run tauri:dev", "/nonexistent/path/xyz"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_port_returns_bindable_port() {
        let p = free_port_from(28000);
        assert!(p >= 1024);
        // the returned port must be bindable right now
        assert!(std::net::TcpListener::bind(("127.0.0.1", p)).is_ok());
    }
}
