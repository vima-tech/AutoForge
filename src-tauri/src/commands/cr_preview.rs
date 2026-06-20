//! Per-CR worktree preview server (方案 A).
//!
//! Unlike `dev_server.rs` — which runs the project's *main* repo — this starts a
//! dev server inside the change-request's **worktree**, so the "本次改动" preview
//! reflects the CR's actual code. Supports two project kinds:
//!   - `web`   → start the dev command, embed `url` in an iframe.
//!   - `tauri` → start the web frontend dev server for the iframe, and optionally
//!               launch the native desktop window via `app_command` (escape hatch D).
//!
//! When the project has no preview config the status is `no_config` / kind `none`,
//! and the frontend collapses the preview area (fallback C).

use crate::core::git::GitProxy;
use crate::models::change_request::ChangeRequest;
use crate::models::project::Project;
use crate::models::worktree::WorktreeSession;
use crate::state::{worktrees_base, AppState, ChildHandle, DevServerHandle};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

#[derive(Debug, Deserialize)]
struct ProjectPreviewConfig {
    dev: Option<DevSpec>,
}

/// `dev:` block of the project's `config_yaml`. All fields optional so legacy
/// configs keep parsing; `kind` defaults to `web`. A legacy `url:` is simply
/// ignored — preview always targets `http://localhost:{port}`.
/// `{port}` placeholders in `command` are substituted per-CR.
#[derive(Debug, Clone, Default, Deserialize)]
struct DevSpec {
    kind: Option<String>,
    command: Option<String>,
    app_command: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CrPreviewStatus {
    pub cr_id: String,
    /// "web" | "tauri" | "none"
    pub kind: String,
    /// "no_config" | "no_session" | "idle" | "starting" | "running" | "stopped"
    pub status: String,
    pub url: Option<String>,
    pub can_launch_app: bool,
    /// True for `tauri` projects: the iframe only renders the web frontend, so
    /// `invoke()` IPC is unavailable and data-backed screens stay empty. The real
    /// preview is the native desktop window (`launch_cr_app`).
    pub frontend_only: bool,
    /// True when `kind` was inferred from the project's files (no explicit
    /// `dev.kind` in config_yaml) — used by the UI to hint the auto-detection.
    pub auto_detected: bool,
    /// True when the native desktop app launched via `launch_cr_app`
    /// (key `cr:app:{cr_id}`) is still alive — lets the UI flip「启动」→「停止」。
    pub app_running: bool,
}

/// Whether the desktop app spawned by `launch_cr_app` for this CR is still alive.
async fn cr_app_running(state: &AppState, cr_id: &str) -> bool {
    let key = format!("cr:app:{cr_id}");
    let child_arc = {
        let servers = state.dev_servers.lock().await;
        servers.get(&key).map(|h| h.child.clone())
    };
    match child_arc {
        Some(arc) => {
            let mut g = arc.lock().await;
            matches!(g.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
        }
        None => false,
    }
}

/// Parse the raw `dev:` block without requiring a command — detection can fill the
/// command in later, so we keep partial/empty specs around.
fn parse_dev_spec_raw(config_yaml: Option<&str>) -> Option<DevSpec> {
    let cfg: ProjectPreviewConfig = serde_yaml::from_str(config_yaml?).ok()?;
    cfg.dev
}

/// What we can infer about a project's framework by sniffing its files.
#[derive(Debug, Clone, Default)]
struct Detected {
    is_tauri: bool,
    /// npm script name for the frontend dev server (iframe target).
    dev_script: Option<String>,
    /// npm script name that launches the native Tauri window.
    tauri_script: Option<String>,
}

/// Detect the project framework from files in `dir` (a repo root or CR worktree).
/// Recognises Tauri via `src-tauri/{tauri.conf.json,Cargo.toml}` and reads
/// `package.json` scripts to find the frontend-dev and tauri-dev commands so a
/// preview works even when `config_yaml` omits them.
fn detect_framework(dir: &str) -> Detected {
    let base = std::path::Path::new(dir);
    let mut d = Detected {
        is_tauri: base.join("src-tauri/tauri.conf.json").exists()
            || base.join("src-tauri/Cargo.toml").exists(),
        ..Default::default()
    };

    if let Ok(pkg) = std::fs::read_to_string(base.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg) {
            if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
                // A script that drives `tauri dev` launches the native window.
                for (name, val) in scripts {
                    let v = val.as_str().unwrap_or("");
                    if v.contains("tauri") && v.contains("dev") {
                        d.tauri_script = Some(name.clone());
                        d.is_tauri = true;
                        break;
                    }
                }
                // Frontend dev server: prefer a literal `dev` script, else the first
                // non-tauri script that runs vite/dev.
                if scripts.contains_key("dev") {
                    d.dev_script = Some("dev".to_string());
                } else {
                    for (name, val) in scripts {
                        let v = val.as_str().unwrap_or("");
                        if (v.contains("vite") || v.contains("dev")) && !v.contains("tauri") {
                            d.dev_script = Some(name.clone());
                            break;
                        }
                    }
                }
            }
        }
    }
    d
}

/// A fully-resolved preview spec: explicit `config_yaml` values win, and anything
/// missing is filled from framework detection (`dir`). Returns `None` only when
/// preview is impossible (no command and nothing detectable) or explicitly off.
#[derive(Debug, Clone)]
struct EffectiveSpec {
    kind: String,
    command: String,
    /// URL template (may contain `{port}`).
    url: String,
    app_command: Option<String>,
    can_launch_app: bool,
    frontend_only: bool,
    auto_detected: bool,
}

fn effective_spec(config_yaml: Option<&str>, dir: &str) -> Option<EffectiveSpec> {
    let explicit = parse_dev_spec_raw(config_yaml);
    let det = detect_framework(dir);

    let explicit_kind = explicit.as_ref().and_then(|s| s.kind.as_deref());
    // Explicit kind always wins; `none` disables preview. When unset, fall back to
    // detection — this is what stops a Tauri project from being shown as plain web.
    let (kind, auto_detected) = match explicit_kind {
        Some("tauri") => ("tauri".to_string(), false),
        Some("web") => ("web".to_string(), false),
        Some("none") => return None,
        _ => (
            if det.is_tauri { "tauri".to_string() } else { "web".to_string() },
            det.is_tauri,
        ),
    };

    let command = explicit
        .as_ref()
        .and_then(|s| s.command.clone())
        .filter(|c| !c.trim().is_empty())
        .or_else(|| det.dev_script.clone().map(|s| format!("npm run {s}")))
        // 非 npm 栈（Java/Go/Python 后端、静态站）的兜底：用栈画像建议的 dev 命令，
        // 使这些项目也能启动预览（端口探活），而不是直接 no_config。
        .or_else(|| crate::core::stack::suggest_run_config(std::path::Path::new(dir)).dev_command)?;

    // Preview URL is fixed to localhost on the auto-allocated port (no longer configurable).
    let url = "http://localhost:{port}".to_string();

    let app_command = explicit
        .as_ref()
        .and_then(|s| s.app_command.clone())
        .filter(|c| !c.trim().is_empty())
        .or_else(|| {
            if kind == "tauri" {
                det.tauri_script.clone().map(|s| format!("npm run {s}"))
            } else {
                None
            }
        });

    let can_launch_app = kind == "tauri" && app_command.is_some();
    let frontend_only = kind == "tauri";
    Some(EffectiveSpec {
        kind,
        command,
        url,
        app_command,
        can_launch_app,
        frontend_only,
        auto_detected,
    })
}

/// Pick the directory to sniff for framework detection: the CR's worktree (its
/// actual code) when present, else the project's main repo.
fn detect_dir<'a>(project: &'a Project, session: Option<&'a WorktreeSession>) -> &'a str {
    session
        .map(|s| s.worktree_path.as_str())
        .unwrap_or(project.repo_path.as_str())
}

/// Deterministic per-session port in [19000, 23000) so re-opening a CR reuses the
/// same port and concurrent CRs don't collide.
fn derive_port(seed: &str) -> u16 {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    19000 + (h % 4000) as u16
}

fn apply_port(template: &str, port: u16) -> String {
    template.replace("{port}", &port.to_string())
}

fn preview_log_path(key: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("autoforge-cr-preview-{key}.log"))
}

/// Open the log file for a preview run and return `Stdio` handles for the child's
/// stdout+stderr, seeding it with the command / cwd / PATH so启动失败 is diagnosable.
fn log_stdio(path: &std::path::Path, command: &str, cwd: &str) -> Result<(Stdio, Stdio), String> {
    let mut file = std::fs::File::create(path).map_err(|e| format!("无法创建日志文件: {e}"))?;
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "$ {}\n# cwd: {}\n# PATH={}\n----------------------------------------",
            command,
            cwd,
            std::env::var("PATH").unwrap_or_default()
        );
    }
    let err = file.try_clone().map_err(|e| format!("日志文件克隆失败: {e}"))?;
    Ok((Stdio::from(file), Stdio::from(err)))
}

/// Tail of the per-CR preview log (stdout+stderr). Empty when nothing ran yet.
#[tauri::command]
pub async fn get_cr_preview_log(cr_id: String) -> Result<String, String> {
    match std::fs::read_to_string(preview_log_path(&cr_id)) {
        Ok(s) if s.len() > 16000 => Ok(s[s.len() - 16000..].to_string()),
        Ok(s) => Ok(s),
        Err(_) => Ok(String::new()),
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

/// Load the CR's project + most recent worktree session.
async fn load_ctx(
    db: &crate::db::Db,
    cr_id: &str,
) -> Result<(Project, Option<WorktreeSession>), String> {
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("cr {cr_id} not found"))?;
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&cr.project_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;
    let session = sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    // 合并后 worktree 目录已被清理，但 DB 行可能仍在；目录不存在则视作无会话，
    // 这样 get_cr_preview 报 no_session（前端隐藏「本次改动」启动行），
    // 框架探测也回落到主仓库（detect_dir）。
    .filter(|s| std::path::Path::new(&s.worktree_path).exists());
    Ok((project, session))
}

#[tauri::command]
pub async fn get_cr_preview(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<CrPreviewStatus, String> {
    let (project, session) = load_ctx(&state.db, &cr_id).await?;

    let Some(spec) = effective_spec(crate::commands::run_config::effective_config(&project).as_deref(), detect_dir(&project, session.as_ref())) else {
        return Ok(CrPreviewStatus {
            cr_id,
            kind: "none".to_string(),
            status: "no_config".to_string(),
            url: None,
            can_launch_app: false,
            frontend_only: false,
            auto_detected: false,
            app_running: false,
        });
    };
    let kind = spec.kind.clone();
    let can_launch_app = spec.can_launch_app;
    let frontend_only = spec.frontend_only;
    let auto_detected = spec.auto_detected;
    let app_running = cr_app_running(&state, &cr_id).await;
    let key = format!("cr:{cr_id}");

    // If a server is already tracked for this CR, report its live status.
    let handle_info: Option<(ChildHandle, String)> = {
        let servers = state.dev_servers.lock().await;
        servers.get(&key).map(|h| (h.child.clone(), h.url.clone()))
    };
    if let Some((child_arc, url)) = handle_info {
        let running = {
            let mut guard = child_arc.lock().await;
            match guard.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        };
        if running {
            let reachable = url_reachable(&url).await;
            return Ok(CrPreviewStatus {
                cr_id,
                kind,
                status: if reachable { "running" } else { "starting" }.to_string(),
                url: Some(url),
                can_launch_app,
                frontend_only,
                auto_detected,
                app_running,
            });
        }
        state.dev_servers.lock().await.remove(&key);
        return Ok(CrPreviewStatus {
            cr_id,
            kind,
            status: "stopped".to_string(),
            url: Some(url),
            can_launch_app,
            frontend_only,
            auto_detected,
            app_running,
        });
    }

    Ok(CrPreviewStatus {
        cr_id,
        kind,
        status: if session.is_some() { "idle" } else { "no_session" }.to_string(),
        url: None,
        can_launch_app,
        frontend_only,
        auto_detected,
        app_running,
    })
}

#[tauri::command]
pub async fn start_cr_preview(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<CrPreviewStatus, String> {
    let (project, session) = load_ctx(&state.db, &cr_id).await?;
    let session = session.ok_or_else(|| "该变更尚无 worktree 会话（实现未开始或已清理）".to_string())?;
    let spec = effective_spec(crate::commands::run_config::effective_config(&project).as_deref(), &session.worktree_path)
        .ok_or_else(|| "未配置预览启动命令（config_yaml 缺少 dev.command，且未能自动识别框架）".to_string())?;

    // Auto-allocate a free port (avoids colliding with other CR previews or a fixed
    // dev port like Vite's 1420); inject it into the command + align the URL.
    let port = crate::commands::dev_server::free_port_from(derive_port(&session.id));
    let cmd_t = spec.command.clone();
    let command = crate::commands::dev_server::inject_port(&cmd_t, port);
    let url = {
        let u = spec.url.clone();
        if u.contains("{port}") {
            apply_port(&u, port)
        } else if u.trim().is_empty() || command != cmd_t {
            format!("http://localhost:{port}")
        } else {
            u
        }
    };
    let kind = spec.kind.clone();
    let can_launch_app = spec.can_launch_app;
    let frontend_only = spec.frontend_only;
    let auto_detected = spec.auto_detected;
    let key = format!("cr:{cr_id}");

    // Kill any existing server for this CR before respawning.
    {
        let mut servers = state.dev_servers.lock().await;
        if let Some(old) = servers.remove(&key) {
            if let Some(child) = old.child.lock().await.as_mut() {
                crate::commands::dev_server::kill_child_group(child).await;
            }
        }
    }

    let (out, err) = log_stdio(&preview_log_path(&cr_id), &command, &session.worktree_path)?;
    let mut cmd = crate::core::platform::shell(&command);
    cmd.current_dir(&session.worktree_path)
        .env("PORT", port.to_string())
        .stdout(out)
        .stderr(err);
    // Own process group so the whole preview tree (shell → vite → node …) can be
    // torn down together instead of orphaning grandchildren.
    crate::core::platform::detach_process_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;

    state.dev_servers.lock().await.insert(
        key,
        DevServerHandle {
            child: Arc::new(Mutex::new(Some(child))),
            url: url.clone(),
        },
    );

    // Replace the placeholder preview_url recorded at execution time with the real
    // running URL, so listPreviewEnvironments consumers stay consistent.
    let _ = sqlx::query(
        "UPDATE preview_environments SET preview_url=?, status='building' WHERE worktree_session_id=?",
    )
    .bind(&url)
    .bind(&session.id)
    .execute(&state.db)
    .await;

    Ok(CrPreviewStatus {
        cr_id,
        kind,
        status: "starting".to_string(),
        url: Some(url),
        can_launch_app,
        frontend_only,
        auto_detected,
        app_running: false,
    })
}

#[tauri::command]
pub async fn stop_cr_preview(cr_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut servers = state.dev_servers.lock().await;
    for key in [format!("cr:{cr_id}"), format!("cr:app:{cr_id}")] {
        if let Some(handle) = servers.remove(&key) {
            if let Some(child) = handle.child.lock().await.as_mut() {
                crate::commands::dev_server::kill_child_group(child).await;
            }
        }
    }
    Ok(())
}

/// Escape hatch D for `tauri` projects: launch the native desktop window
/// (`dev.app_command`) in the worktree. The iframe can only show the web
/// frontend; this opens the real shell. Tracked so `stop_cr_preview` can kill it.
#[tauri::command]
pub async fn launch_cr_app(cr_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let (project, session) = load_ctx(&state.db, &cr_id).await?;
    let session = session.ok_or_else(|| "无 worktree 会话".to_string())?;
    let spec = effective_spec(crate::commands::run_config::effective_config(&project).as_deref(), &session.worktree_path)
        .ok_or_else(|| "未配置预览".to_string())?;
    let app_cmd = spec
        .app_command
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| "未配置桌面应用启动命令（dev.app_command），且未能自动识别 tauri dev 脚本".to_string())?;

    let key = format!("cr:app:{cr_id}");
    {
        let mut servers = state.dev_servers.lock().await;
        if let Some(old) = servers.remove(&key) {
            if let Some(child) = old.child.lock().await.as_mut() {
                crate::commands::dev_server::kill_child_group(child).await;
            }
        }
    }

    // 分配独立空闲端口并注入（PORT 环境变量 + 命令内 {port} 占位）。必须做：
    // tauri 应用的 beforeDevCommand（vite，strictPort:true）默认固定 1420，
    // 与正在运行的主 AutoForge 实例冲突时会直接报「Port 1420 is already in use」
    // 而中止启动，桌面窗口起不来。用 app: 前缀的 seed 避免与同 CR 的 web 预览撞端口。
    let port =
        crate::commands::dev_server::free_port_from(derive_port(&format!("app:{}", session.id)));
    let app_cmd = crate::commands::dev_server::inject_port(&app_cmd, port);

    // 写入与「查看启动日志」按钮一致的日志路径（get_cr_preview_log 读 cr_id），
    // 否则启动失败时用户在 UI 看不到任何日志。
    let (out, err) = log_stdio(&preview_log_path(&cr_id), &app_cmd, &session.worktree_path)?;
    let mut cmd = crate::core::platform::shell(&app_cmd);
    cmd.current_dir(&session.worktree_path)
        // 隔离 DB，避免与主 AutoForge 共享生产库导致迁移不一致 panic。
        .envs(isolated_app_env(&session.worktree_path))
        .env("PORT", port.to_string())
        .stdout(out)
        .stderr(err);
    // Own process group so the launched app tree can be torn down together.
    crate::core::platform::detach_process_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;

    state.dev_servers.lock().await.insert(
        key,
        DevServerHandle {
            child: Arc::new(Mutex::new(Some(child))),
            url: String::new(),
        },
    );
    Ok(())
}

// ── 分支启动（左侧「启动项目」选分支）────────────────────────────────────────
//
// 在项目本地分支上启动预览：每个 (project, branch) 用一个**独立 worktree**（detached
// 到分支 tip，避免与主仓库/其它 worktree 的「已检出」冲突），并把主仓库的
// `node_modules` 软链进去，免去重复安装。支持多分支并行（state.dev_servers 以
// `branch:<project>:<branch>` 为键）。web → dev server；tauri → 启动桌面应用。

async fn load_project(db: &crate::db::Db, project_id: &str) -> Result<Project, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_main: bool,
    pub is_dev: bool,
}

/// 列出项目本地分支（含当前/main/dev 标记），供左侧下拉选择。
#[tauri::command]
pub async fn list_local_branches(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<BranchInfo>, String> {
    let project = load_project(&state.db, &project_id).await?;
    let git = GitProxy::new(&project.repo_path);
    let current = git
        .run_str(&["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (_code, out, err) = git
        .run(&["branch", "--format=%(refname:short)"])
        .await
        .map_err(|e| e.to_string())?;
    if out.trim().is_empty() && !err.trim().is_empty() {
        return Err(format!("读取分支失败: {err}"));
    }
    let branches = out
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.contains("HEAD detached") && !l.contains("(no branch)"))
        .map(|name| BranchInfo {
            name: name.to_string(),
            is_current: name == current,
            is_main: name == project.branch_main,
            is_dev: name == project.branch_dev,
        })
        .collect();
    Ok(branches)
}

#[derive(Debug, Serialize)]
pub struct BranchPreviewStatus {
    pub branch: String,
    /// "web" | "tauri"
    pub kind: String,
    /// "starting" | "running"
    pub status: String,
    pub url: Option<String>,
    pub can_launch_app: bool,
}

fn sanitize_branch(b: &str) -> String {
    b.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn branch_wt_path(project_id: &str, branch: &str) -> String {
    format!("{}/branch/{}/{}", worktrees_base(), project_id, sanitize_branch(branch))
}

fn branch_log_key(project_id: &str, branch: &str) -> String {
    format!("branch-{}-{}", project_id, sanitize_branch(branch))
}

/// 给预览用的 Tauri 应用一套**隔离的 XDG 目录**，使其 `app_data_dir()` 落到 worktree 内的
/// 独立路径，用自己的 SQLite DB 跑自己的迁移——而不是共享主 AutoForge 的生产库。
/// 否则：分支的迁移集与生产库已应用的迁移不一致会直接 panic（migration X missing），
/// 且多实例并发写同一 SQLite 也不安全。Linux 上 `app_data_dir` 走 XDG_DATA_HOME，
/// 故无需改被启动应用的代码即可隔离。
fn isolated_app_env(worktree: &str) -> Vec<(String, String)> {
    let home = format!("{worktree}/.preview-home");
    let mut envs = Vec::new();
    for (key, sub) in [
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        let dir = format!("{home}/{sub}");
        let _ = std::fs::create_dir_all(&dir);
        envs.push((key.to_string(), dir));
    }
    envs
}

/// Ensure a reusable detached worktree at `branch`'s tip, with `node_modules`
/// symlinked from the main repo so the dev server can start without installing.
async fn ensure_branch_worktree(repo_path: &str, branch: &str, wt_path: &str) -> Result<(), String> {
    // 已存在则复用（决策：stop 不删 worktree，便于快速重启）。
    if std::path::Path::new(wt_path).join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = std::path::Path::new(wt_path).parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let git = GitProxy::new(repo_path);
    let _ = git.run(&["worktree", "prune"]).await;
    // `--detach`：检出到分支 tip 但不占用分支引用，避免「branch already checked out」。
    let (code, _out, err) = git
        .run(&["worktree", "add", "--detach", wt_path, branch])
        .await
        .map_err(|e| e.to_string())?;
    if code != 0 {
        return Err(format!("git worktree add 失败: {err}"));
    }
    // 软链依赖缓存目录（gitignore 的 node_modules 等不在 worktree 内），免重复安装。
    // 由栈画像决定要软链哪些目录：前端/Tauri/Node → node_modules；Java/Go/Python
    // 走全局缓存（~/.m2、GOMODCACHE、pip cache），不在仓库内故无需软链。
    #[cfg(unix)]
    {
        for rel in crate::core::stack::dep_cache_dirs(std::path::Path::new(repo_path)) {
            let src = std::path::Path::new(repo_path).join(&rel);
            let dst = std::path::Path::new(wt_path).join(&rel);
            if src.exists() && !dst.exists() {
                let _ = std::os::unix::fs::symlink(&src, &dst);
            }
        }
    }
    Ok(())
}

/// 启动指定分支的预览（web dev server 或 tauri 桌面应用），随机空闲端口避冲突。
#[tauri::command]
pub async fn start_branch_preview(
    project_id: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<BranchPreviewStatus, String> {
    let project = load_project(&state.db, &project_id).await?;
    let wt_path = branch_wt_path(&project_id, &branch);
    ensure_branch_worktree(&project.repo_path, &branch, &wt_path).await?;

    let spec = effective_spec(crate::commands::run_config::effective_config(&project).as_deref(), &wt_path)
        .ok_or_else(|| "未配置预览启动命令（config_yaml 缺少 dev.command，且未能自动识别框架）".to_string())?;
    let kind = spec.kind.clone();
    let can_launch_app = spec.can_launch_app;

    let key = format!("branch:{project_id}:{branch}");
    let port = crate::commands::dev_server::free_port_from(derive_port(&key));
    // tauri → 启动桌面应用（app_command）；web → dev server。
    let raw_cmd = if kind == "tauri" {
        spec.app_command
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| spec.command.clone())
    } else {
        spec.command.clone()
    };
    let command = crate::commands::dev_server::inject_port(&raw_cmd, port);
    // tauri 应用无 iframe URL；web 用本地端口 URL。
    let url = if kind == "tauri" {
        String::new()
    } else {
        format!("http://localhost:{port}")
    };

    {
        let mut servers = state.dev_servers.lock().await;
        if let Some(old) = servers.remove(&key) {
            if let Some(child) = old.child.lock().await.as_mut() {
                crate::commands::dev_server::kill_child_group(child).await;
            }
        }
    }

    let (out, err) = log_stdio(
        &preview_log_path(&branch_log_key(&project_id, &branch)),
        &command,
        &wt_path,
    )?;
    let mut cmd = crate::core::platform::shell(&command);
    cmd.current_dir(&wt_path)
        .env("PORT", port.to_string())
        .envs(isolated_app_env(&wt_path))
        .stdout(out)
        .stderr(err);
    crate::core::platform::detach_process_group(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;

    state.dev_servers.lock().await.insert(
        key,
        DevServerHandle {
            child: Arc::new(Mutex::new(Some(child))),
            url: url.clone(),
        },
    );

    Ok(BranchPreviewStatus {
        branch,
        kind,
        status: "starting".to_string(),
        url: if url.is_empty() { None } else { Some(url) },
        can_launch_app,
    })
}

/// 列出当前项目所有正在运行的分支预览（已退出的句柄顺带清理）。
#[tauri::command]
pub async fn list_branch_previews(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<BranchPreviewStatus>, String> {
    let project = load_project(&state.db, &project_id).await?;
    let kind = effective_spec(crate::commands::run_config::effective_config(&project).as_deref(), &project.repo_path)
        .map(|s| s.kind)
        .unwrap_or_else(|| "web".to_string());
    let prefix = format!("branch:{project_id}:");

    let entries: Vec<(String, ChildHandle, String)> = {
        let servers = state.dev_servers.lock().await;
        servers
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, h)| (k.clone(), h.child.clone(), h.url.clone()))
            .collect()
    };

    let mut out = Vec::new();
    let mut dead = Vec::new();
    for (key, child_arc, url) in entries {
        let running = {
            let mut g = child_arc.lock().await;
            match g.as_mut() {
                Some(c) => matches!(c.try_wait(), Ok(None)),
                None => false,
            }
        };
        if !running {
            dead.push(key);
            continue;
        }
        let branch = key.strip_prefix(&prefix).unwrap_or("").to_string();
        let is_app = url.is_empty();
        let status = if is_app || url_reachable(&url).await {
            "running"
        } else {
            "starting"
        };
        out.push(BranchPreviewStatus {
            branch,
            kind: kind.clone(),
            status: status.to_string(),
            url: if url.is_empty() { None } else { Some(url) },
            can_launch_app: is_app,
        });
    }
    if !dead.is_empty() {
        let mut servers = state.dev_servers.lock().await;
        for k in dead {
            servers.remove(&k);
        }
    }
    out.sort_by(|a, b| a.branch.cmp(&b.branch));
    Ok(out)
}

/// 停止某分支预览（仅杀进程，保留 worktree 以便快速重启）。
#[tauri::command]
pub async fn stop_branch_preview(
    project_id: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let key = format!("branch:{project_id}:{branch}");
    let mut servers = state.dev_servers.lock().await;
    if let Some(handle) = servers.remove(&key) {
        if let Some(child) = handle.child.lock().await.as_mut() {
            crate::commands::dev_server::kill_child_group(child).await;
        }
    }
    Ok(())
}

/// 某分支预览的启动日志尾部。
#[tauri::command]
pub async fn get_branch_preview_log(project_id: String, branch: String) -> Result<String, String> {
    match std::fs::read_to_string(preview_log_path(&branch_log_key(&project_id, &branch))) {
        Ok(s) if s.len() > 16000 => Ok(s[s.len() - 16000..].to_string()),
        Ok(s) => Ok(s),
        Err(_) => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_deterministic_and_in_range() {
        let a = derive_port("session-xyz");
        assert_eq!(a, derive_port("session-xyz"));
        assert!((19000..23000).contains(&a));
    }

    #[test]
    fn port_template_substitution() {
        assert_eq!(apply_port("npm run dev -- --port {port}", 19001), "npm run dev -- --port 19001");
        assert_eq!(apply_port("http://localhost:{port}", 20000), "http://localhost:20000");
        assert_eq!(apply_port("no placeholder", 19000), "no placeholder");
    }

    // A throwaway dir that exists but holds no framework markers, so detection is
    // inert and we can test the pure config-driven resolution path.
    fn empty_dir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("af-cr-empty-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Build a temp project dir with the given `tauri.conf.json` presence and
    /// `package.json` scripts, returning its path (caller cleans up).
    fn make_project(tag: &str, is_tauri: bool, scripts: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("af-cr-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if is_tauri {
            std::fs::create_dir_all(root.join("src-tauri")).unwrap();
            std::fs::write(root.join("src-tauri/tauri.conf.json"), "{}").unwrap();
        }
        std::fs::write(
            root.join("package.json"),
            format!("{{\"scripts\":{{{scripts}}}}}"),
        )
        .unwrap();
        root
    }

    #[test]
    fn kind_defaults_to_web_without_config_or_markers() {
        let dir = empty_dir();
        // No config, no detectable framework → no command → no preview.
        assert!(effective_spec(None, dir.to_str().unwrap()).is_none());
        // Explicit web command, inert dir → plain web preview, no native launch.
        let yaml = "dev:\n  command: \"npm run dev\"\n  url: \"http://localhost:{port}\"\n";
        let spec = effective_spec(Some(yaml), dir.to_str().unwrap()).unwrap();
        assert_eq!(spec.kind, "web");
        assert!(!spec.frontend_only);
        assert!(!spec.can_launch_app);
        assert!(!spec.auto_detected);
    }

    #[test]
    fn detects_tauri_and_infers_app_command() {
        let root = make_project(
            "tauri",
            true,
            "\"dev\":\"vite\",\"tauri:dev\":\"tauri dev\"",
        );
        let det = detect_framework(root.to_str().unwrap());
        assert!(det.is_tauri);
        assert_eq!(det.dev_script.as_deref(), Some("dev"));
        assert_eq!(det.tauri_script.as_deref(), Some("tauri:dev"));

        // No explicit kind in config → detection promotes it to tauri + infers
        // the native launch command, so the iframe is flagged frontend-only.
        let spec = effective_spec(None, root.to_str().unwrap()).unwrap();
        assert_eq!(spec.kind, "tauri");
        assert_eq!(spec.command, "npm run dev");
        assert_eq!(spec.app_command.as_deref(), Some("npm run tauri:dev"));
        assert!(spec.can_launch_app);
        assert!(spec.frontend_only);
        assert!(spec.auto_detected);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_kind_wins_over_detection() {
        let root = make_project("forceweb", true, "\"dev\":\"vite\"");
        // A tauri repo, but the user pinned kind: web → respect it (not auto).
        let yaml = "dev:\n  kind: \"web\"\n  command: \"npm run dev\"\n";
        let spec = effective_spec(Some(yaml), root.to_str().unwrap()).unwrap();
        assert_eq!(spec.kind, "web");
        assert!(!spec.frontend_only);
        assert!(!spec.auto_detected);
        let _ = std::fs::remove_dir_all(&root);
    }
}
