//! Per-CR worktree preview server (方案 A).
//!
//! Unlike `dev_server.rs` — which runs the project's *main* repo — this starts a
//! dev server inside the change-request's **worktree**, so the "本次改动" preview
//! reflects the CR's actual code. Supports these project kinds:
//!   - `web`     → start the dev command, embed `url` in an iframe.
//!   - `tauri`   → start the web frontend dev server for the iframe, and optionally
//!     launch the native desktop window via `app_command` (escape hatch D).
//!   - `miniapp` → 微信小程序：无可 iframe 的 localhost server，预览=一次性编译产物
//!     （`build_cr_miniapp` run-to-completion，不进 dev_servers、不分配端口、不探活）。
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
    /// "web" | "tauri" | "miniapp" | "none"
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
    let suggestion = crate::core::stack::suggest_run_config(std::path::Path::new(dir));

    let explicit_kind = explicit.as_ref().and_then(|s| s.kind.as_deref());
    // Explicit kind always wins; `none` disables preview. When unset, fall back to
    // detection — this is what stops a Tauri project from being shown as plain web,
    // and a 微信小程序 from being shown as a (non-existent) localhost web server.
    let (kind, auto_detected) = match explicit_kind {
        Some("tauri") => ("tauri".to_string(), false),
        Some("web") => ("web".to_string(), false),
        Some("miniapp") => ("miniapp".to_string(), false),
        Some("none") => return None,
        _ => {
            if det.is_tauri {
                ("tauri".to_string(), true)
            } else if suggestion.dev_kind.as_deref() == Some("miniapp") {
                ("miniapp".to_string(), true)
            } else {
                ("web".to_string(), false)
            }
        }
    };

    // 小程序：预览语义是「一次性编译产物」而非 dev server，命令取 build 而非 dev。
    let command = if kind == "miniapp" {
        explicit
            .as_ref()
            .and_then(|s| s.command.clone())
            .filter(|c| !c.trim().is_empty())
            .or_else(|| suggestion.build_command.clone())
            .or_else(|| suggestion.dev_command.clone())?
    } else {
        explicit
            .as_ref()
            .and_then(|s| s.command.clone())
            .filter(|c| !c.trim().is_empty())
            .or_else(|| det.dev_script.clone().map(|s| format!("npm run {s}")))
            // 非 npm 栈（Java/Go/Python 后端、静态站）的兜底：用栈画像建议的 dev 命令，
            // 使这些项目也能启动预览（端口探活），而不是直接 no_config。
            .or_else(|| suggestion.dev_command.clone())?
    };

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

/// 微信小程序编译产物（一次性 build 的结果）。区别于 dev server：无持久进程、无端口、无探活。
#[derive(Debug, Serialize)]
pub struct MiniappBuildResult {
    pub cr_id: String,
    pub success: bool,
    /// 进程退出码（正常退出取实际码；被信号杀死等异常取 -1）。
    pub exit_code: i32,
    /// 编译产物目录（相对 worktree 根，探测常见输出目录得到）；未找到则 None。
    pub artifact_dir: Option<String>,
    /// 执行的编译命令（已注入实际参数）。
    pub command: String,
    /// 档位 2：是否已用微信开发者工具 CLI 自动打开产物目录（未配置 CLI / 拉起失败 → false）。
    pub launched_devtools: bool,
}

/// 编译微信小程序 CR：**一次性跑 build 命令到结束**，不像 dev server 那样长驻/探活。
/// stdout+stderr 写入与 web 预览同一份日志文件（前端 `start_preview_log_tail` 可实时订阅），
/// 退出后探测产物目录返回。产物可用微信开发者工具打开（档位 1：手动；档位 2 见 §3.3 CLI 拉起）。
#[tauri::command]
pub async fn build_cr_miniapp(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<MiniappBuildResult, String> {
    let (project, session) = load_ctx(&state.db, &cr_id).await?;
    let session = session.ok_or_else(|| "该变更尚无 worktree 会话（实现未开始或已清理）".to_string())?;
    let spec = effective_spec(
        crate::commands::run_config::effective_config(&project).as_deref(),
        &session.worktree_path,
    )
    .ok_or_else(|| "未能识别小程序工程或缺少编译命令".to_string())?;
    if spec.kind != "miniapp" {
        return Err("当前变更不是微信小程序工程（预览类型非 miniapp）".to_string());
    }

    let command = spec.command.clone();
    // 复用 web 预览的日志文件路径，前端日志订阅无需区分。写入命令头便于诊断。
    let (out, err) = log_stdio(&preview_log_path(&cr_id), &command, &session.worktree_path)?;
    let mut cmd = crate::core::platform::shell(&command);
    cmd.current_dir(&session.worktree_path)
        .stdout(out)
        .stderr(err);
    crate::core::platform::detach_process_group(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| format!("启动编译失败: {e}"))?;
    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待编译进程失败: {e}"))?;
    let exit_code = status.code().unwrap_or(-1);
    let success = status.success();

    // 探测常见小程序产物目录（Taro: dist；uni-app: dist/build/mp-weixin / unpackage/...；mpx: dist/wx）。
    let artifact_dir = if success {
        find_miniapp_artifact(&session.worktree_path)
    } else {
        None
    };

    // 档位 2：若配置了微信开发者工具 CLI 路径，编译成功后自动用它打开产物目录。
    // best-effort：未配置 / 路径不存在 / 拉起失败都静默降级到档位 1（手动打开），绝不报硬错。
    let launched_devtools = match (&artifact_dir, success) {
        (Some(rel), true) => {
            let abs = std::path::Path::new(&session.worktree_path).join(rel);
            launch_devtools(&state, &abs).await
        }
        _ => false,
    };

    Ok(MiniappBuildResult {
        cr_id,
        success,
        exit_code,
        artifact_dir,
        command,
        launched_devtools,
    })
}

/// 档位 2：用微信开发者工具 CLI 打开产物目录（`<cli> open --project <abs>`）。
/// 读 app_settings 的 `miniapp.devtools_cli_path`；未配置 / 文件不存在 / spawn 失败 → 返回 false（降级档位 1）。
async fn launch_devtools(state: &AppState, artifact_abs: &std::path::Path) -> bool {
    let cli = match sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key='miniapp.devtools_cli_path'",
    )
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(p)) if !p.trim().is_empty() => p.trim().to_string(),
        _ => return false,
    };
    if !std::path::Path::new(&cli).exists() {
        return false;
    }
    let mut cmd = tokio::process::Command::new(&cli);
    cmd.arg("open")
        .arg("--project")
        .arg(artifact_abs)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::core::platform::detach_process_group(&mut cmd);
    cmd.spawn().is_ok()
}

/// 探测微信小程序编译产物目录（返回相对 worktree 根的路径）。按常见框架输出约定逐一探测。
fn find_miniapp_artifact(worktree: &str) -> Option<String> {
    let root = std::path::Path::new(worktree);
    const CANDIDATES: &[&str] = &[
        "dist/build/mp-weixin",      // uni-app (vue-cli)
        "unpackage/dist/build/mp-weixin", // uni-app (HBuilderX/vite)
        "dist/weapp",                // Taro (可配置)
        "dist/wx",                   // mpx
        "dist",                      // Taro 默认 / 通用兜底
    ];
    CANDIDATES
        .iter()
        .find(|rel| root.join(rel).is_dir())
        .map(|rel| rel.to_string())
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
        // 排除 AutoForge 内部 worktree 分支（execution 创建的 `autoforge/<cr>-i<n>`），
        // 它们只是 CR 实现的临时检出，不是用户可启动的项目分支。
        .filter(|l| {
            !l.is_empty()
                && !l.contains("HEAD detached")
                && !l.contains("(no branch)")
                && !l.starts_with("autoforge/")
        })
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

// ── 预览日志实时 tail（事件驱动）─────────────────────────────────────────────
//
// 预览/dev-server 子进程的 stdout+stderr 直写日志文件、不流经 Rust（见 `log_stdio` /
// `dev_server`），故无法像 code-agent 那样在管道上挂 LogSink。改为：前端打开日志弹窗时
// 启动一个后台 tail 任务，周期读取该日志文件的「新增字节」转成 `AppEvent::PreviewLog`
// 增量推给前端——前端只 append，不再每秒全文重取/重解析/重渲染。子进程仍只写文件，与
// Rust 保持解耦（不被父进程抽水速度牵制）。

const TAIL_INTERVAL_MS: u64 = 600;
/// 首次推送只回带尾部这么多字节，避免超长 build 日志一次性灌爆前端。
const TAIL_INITIAL_CAP: u64 = 64 * 1024;

#[allow(clippy::type_complexity)]
static PREVIEW_TAILERS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>,
> = std::sync::OnceLock::new();

fn preview_tailers(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>> {
    PREVIEW_TAILERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 把前端弹窗 sig 解析成日志文件 key：`cr:<id>` → `<id>`；`branch:<pid>:<branch>` →
/// `branch_log_key`。git refname 不含 `:`，故 `branch:` 后按首个 `:` 切分无歧义。
fn log_key_from_sig(sig: &str) -> Option<String> {
    if let Some(id) = sig.strip_prefix("cr:") {
        return Some(id.to_string());
    }
    if let Some(rest) = sig.strip_prefix("branch:") {
        let (pid, branch) = rest.split_once(':')?;
        return Some(branch_log_key(pid, branch));
    }
    None
}

/// 启动某日志弹窗的实时 tail。重复启动同一 sig 会先停旧任务（处理重订阅/重挂载）。
#[tauri::command]
pub async fn start_preview_log_tail(app: tauri::AppHandle, sig: String) -> Result<(), String> {
    let Some(file_key) = log_key_from_sig(&sig) else {
        return Err(format!("无法解析日志标识: {sig}"));
    };
    let path = preview_log_path(&file_key);

    // 先停掉同 sig 的旧任务，避免重复推送。
    if let Some(old) = preview_tailers().lock().unwrap().remove(&sig) {
        old.abort();
    }

    let task_sig = sig.clone();
    let handle = tokio::spawn(async move {
        let mut offset: u64 = 0;
        let mut first = true;
        // 跨轮保留「未构成完整 UTF-8 字符」的尾部字节，下一轮拼回，避免多字节字符被切碎。
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if len < offset {
                // 文件被重建（新一轮预览覆盖写）→ 从头重读。
                offset = 0;
                first = true;
                pending.clear();
            }
            if len > offset {
                // 首次只取尾部 TAIL_INITIAL_CAP，避免一次性灌爆。
                let trim_lead = first && len - offset > TAIL_INITIAL_CAP;
                let start = if trim_lead { len - TAIL_INITIAL_CAP } else { offset };
                if let Ok(mut f) = std::fs::File::open(&path) {
                    use std::io::{Read, Seek, SeekFrom};
                    let mut buf = Vec::new();
                    if f.seek(SeekFrom::Start(start)).is_ok() && f.read_to_end(&mut buf).is_ok() {
                        offset = len;
                        let mut slice = &buf[..];
                        if trim_lead {
                            // 截断点可能落在多字节字符中间：跳过前导续字节对齐到字符边界。
                            let skip = slice.iter().take_while(|&&b| b & 0xC0 == 0x80).count();
                            slice = &slice[skip..];
                        }
                        pending.extend_from_slice(slice);
                        // 只发送可构成完整 UTF-8 的前缀，残余留到下轮。
                        let valid = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) => e.valid_up_to(),
                        };
                        if valid > 0 {
                            let chunk = String::from_utf8_lossy(&pending[..valid]).into_owned();
                            pending.drain(..valid);
                            crate::core::event::emit(
                                &app,
                                crate::core::event::AppEvent::PreviewLog {
                                    key: task_sig.clone(),
                                    chunk,
                                },
                            );
                        }
                    }
                }
                first = false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(TAIL_INTERVAL_MS)).await;
        }
    });

    preview_tailers().lock().unwrap().insert(sig, handle);
    Ok(())
}

/// 停止某日志弹窗的 tail（前端关闭弹窗时调用）。
#[tauri::command]
pub async fn stop_preview_log_tail(sig: String) -> Result<(), String> {
    if let Some(h) = preview_tailers().lock().unwrap().remove(&sig) {
        h.abort();
    }
    Ok(())
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

    #[test]
    fn miniapp_detected_with_build_command_not_dev_server() {
        // Taro 工程：自动识别为 miniapp，预览命令取 build（非 dev server）。
        let root = std::env::temp_dir().join(format!("af-cr-taro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("project.config.json"), "{}").unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"scripts":{"build:weapp":"taro build --type weapp","dev:weapp":"taro build --type weapp --watch"},"dependencies":{"@tarojs/taro":"^4"}}"#,
        )
        .unwrap();

        let spec = effective_spec(None, root.to_str().unwrap()).unwrap();
        assert_eq!(spec.kind, "miniapp");
        assert_eq!(spec.command, "npm run build:weapp"); // build，不是 dev
        assert!(!spec.can_launch_app);
        assert!(!spec.frontend_only);
        assert!(spec.auto_detected);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_miniapp_artifact_probes_common_dirs() {
        let root = std::env::temp_dir().join(format!("af-cr-artifact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dist/build/mp-weixin")).unwrap();
        assert_eq!(
            find_miniapp_artifact(root.to_str().unwrap()).as_deref(),
            Some("dist/build/mp-weixin")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
