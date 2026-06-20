use crate::core::git::GitProxy;
use crate::models::project::Project;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;
use tracing::{info, warn};

/// Status of a project's working tree relative to `origin/<dev>`, powering the
/// in-app "同步更新" (self-update) control. Only meaningful for the project whose
/// repo is the running app's own source (`is_self_managed`).
#[derive(Debug, Serialize)]
pub struct SelfUpdateStatus {
    pub repo_path: String,
    pub branch: String,
    /// `<dev>` is the branch currently checked out in the main working tree —
    /// i.e. this is AutoForge managing its own live repo.
    pub is_self_managed: bool,
    /// Uncommitted changes are present (a plain pull could be blocked by git).
    pub dirty: bool,
    /// Commits `origin/<dev>` is ahead of the local checkout (pull would bring).
    pub behind: i64,
    /// Local commits not yet on `origin/<dev>`.
    pub ahead: i64,
}

#[derive(Debug, Serialize)]
pub struct SelfUpdateResult {
    pub ok: bool,
    /// Number of commits fast-forwarded in.
    pub pulled: i64,
    pub message: String,
    /// True when source actually changed and the dev process must rebuild/restart.
    pub restart_required: bool,
}

async fn load_project(db: &crate::db::Db, project_id: &str) -> Result<Project, String> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("查询项目失败: {}", e))?
        .ok_or_else(|| format!("项目 {} 不存在", project_id))
}

/// Count commits in `range` (e.g. `HEAD..origin/dev`). Returns 0 on any error.
/// Failures here silently skew behind/ahead (and thus disable the UI button),
/// so every non-happy path is logged for diagnosis.
async fn count_range(git: &GitProxy, range: &str) -> i64 {
    match git.run(&["rev-list", "--count", range]).await {
        Ok((0, out, _)) => match out.trim().parse::<i64>() {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    "self_update: rev-list {} 输出无法解析为数字 ({:?}): {}",
                    range,
                    out.trim(),
                    e
                );
                0
            }
        },
        Ok((code, _, err)) => {
            warn!(
                "self_update: rev-list {} 失败 (code={}): {}",
                range,
                code,
                err.trim()
            );
            0
        }
        Err(e) => {
            warn!("self_update: rev-list {} 执行错误: {}", range, e);
            0
        }
    }
}

/// Fetch `origin/<branch>` and log the outcome. The callers previously discarded
/// this result (`let _ = ...`), which hid network/SSH failures that leave
/// behind/ahead stale — exactly the silent failure mode we need visibility into.
async fn fetch_and_log(git: &GitProxy, branch: &str, ctx: &str) {
    match git.run(&["fetch", "origin", branch]).await {
        Ok((0, _, _)) => info!("self_update[{}]: fetch origin/{} 成功", ctx, branch),
        Ok((code, _, err)) => warn!(
            "self_update[{}]: fetch origin/{} 失败 (code={}): {}",
            ctx,
            branch,
            code,
            err.trim()
        ),
        Err(e) => warn!(
            "self_update[{}]: fetch origin/{} 执行错误: {}",
            ctx, branch, e
        ),
    }
}

/// Behind-count for the self-managed project (the one whose working tree is
/// currently on its `<dev>` branch). Powers the periodic badge poll on the
/// "同步更新" control — cheap: only the matched project is fetched.
#[derive(Debug, Serialize)]
pub struct SelfUpdatePending {
    pub project_id: Option<String>,
    pub behind: i64,
}

#[tauri::command]
pub async fn self_update_pending(
    state: State<'_, AppState>,
) -> Result<SelfUpdatePending, String> {
    let projects = sqlx::query_as::<_, Project>("SELECT * FROM projects")
        .fetch_all(&state.db)
        .await
        .map_err(|e| format!("查询项目失败: {}", e))?;

    info!(
        "self_update_pending: 扫描 {} 个项目寻找自管理仓库",
        projects.len()
    );

    for p in projects {
        let git = GitProxy::new(&p.repo_path);
        let branch = git
            .run(&["branch", "--show-current"])
            .await
            .ok()
            .map(|(_, out, _)| out.trim().to_string())
            .unwrap_or_default();
        if branch.is_empty() || branch != p.branch_dev {
            continue;
        }
        // Self-managed project found — refresh and measure how far behind it is.
        info!(
            "self_update_pending: 命中自管理仓库 {} ({}), 当前分支={}",
            p.name, p.repo_path, branch
        );
        fetch_and_log(&git, &p.branch_dev, "pending").await;
        let behind = count_range(&git, &format!("HEAD..origin/{}", p.branch_dev)).await;
        info!("self_update_pending: 项目 {} 落后 {} 个提交", p.id, behind);
        return Ok(SelfUpdatePending {
            project_id: Some(p.id),
            behind,
        });
    }

    info!("self_update_pending: 未找到自管理仓库（无项目当前停在其 dev 分支）");
    Ok(SelfUpdatePending {
        project_id: None,
        behind: 0,
    })
}

#[tauri::command]
pub async fn self_update_status(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<SelfUpdateStatus, String> {
    let project = load_project(&state.db, &project_id).await?;
    let git = GitProxy::new(&project.repo_path);
    info!(
        "self_update_status: 项目 {} repo={}",
        project_id, project.repo_path
    );

    let branch = git
        .run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default();
    let is_self_managed = !branch.is_empty() && branch == project.branch_dev;
    info!(
        "self_update_status: 当前分支={} branch_dev={} is_self_managed={}",
        if branch.is_empty() { "<空/游离HEAD>" } else { &branch },
        project.branch_dev,
        is_self_managed
    );

    let dirty = git
        .run(&["status", "--porcelain"])
        .await
        .map(|(_, out, _)| !out.trim().is_empty())
        .unwrap_or(false);

    // Best-effort refresh so behind/ahead reflect the real remote tip.
    fetch_and_log(&git, &project.branch_dev, "status").await;
    let remote = format!("origin/{}", project.branch_dev);
    let behind = count_range(&git, &format!("HEAD..{}", remote)).await;
    let ahead = count_range(&git, &format!("{}..HEAD", remote)).await;
    info!(
        "self_update_status: behind={} ahead={} dirty={}",
        behind, ahead, dirty
    );

    Ok(SelfUpdateStatus {
        repo_path: project.repo_path,
        branch,
        is_self_managed,
        dirty,
        behind,
        ahead,
    })
}

#[tauri::command]
pub async fn self_update_pull(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<SelfUpdateResult, String> {
    let project = load_project(&state.db, &project_id).await?;
    let git = GitProxy::new(&project.repo_path);
    info!(
        "self_update_pull: 项目 {} repo={} 开始拉取",
        project_id, project.repo_path
    );

    fetch_and_log(&git, &project.branch_dev, "pull").await;
    let remote = format!("origin/{}", project.branch_dev);
    let behind = count_range(&git, &format!("HEAD..{}", remote)).await;
    info!("self_update_pull: behind={}", behind);
    if behind == 0 {
        info!("self_update_pull: 已是最新，跳过 pull");
        return Ok(SelfUpdateResult {
            ok: true,
            pulled: 0,
            message: "已是最新，无需更新。".to_string(),
            restart_required: false,
        });
    }

    let ahead = count_range(&git, &format!("{}..HEAD", remote)).await;
    info!("self_update_pull: behind={} ahead={}", behind, ahead);

    // First try the conservative fast-forward: a clean repo with no local
    // commits updates with zero risk of rewriting history or losing edits.
    let (code, _, err) = git
        .run(&["pull", "--ff-only", "origin", &project.branch_dev])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    info!(
        "self_update_pull: git pull --ff-only origin {} 退出码={}",
        project.branch_dev, code
    );

    if code == 0 {
        info!("self_update_pull: 成功快进 {} 个提交", behind);
        return Ok(SelfUpdateResult {
            ok: true,
            pulled: behind,
            message: format!(
                "已拉取 {} 个提交。源码已更新，开发模式将自动重新编译并重启以生效。",
                behind
            ),
            restart_required: true,
        });
    }

    warn!(
        "self_update_pull: ff-only 拉取失败 (code={}): {}",
        code,
        err.trim()
    );

    // ff-only failed. If the local branch carries its own commits (ahead>0) the
    // self-managed repo is *diverged* and can never fast-forward — replay those
    // local commits onto the new remote tip via rebase instead. `--autostash`
    // shelves uncommitted edits before the rebase and restores them after, so
    // dirty working trees don't block the update.
    if ahead > 0 {
        info!(
            "self_update_pull: 检测到分叉(本地领先 {} 个提交)，改用 rebase 拉取",
            ahead
        );
        let (rcode, _rout, rerr) = git
            .run(&[
                "pull",
                "--rebase",
                "--autostash",
                "origin",
                &project.branch_dev,
            ])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        info!(
            "self_update_pull: git pull --rebase --autostash origin {} 退出码={}",
            project.branch_dev, rcode
        );

        if rcode == 0 {
            info!(
                "self_update_pull: rebase 拉取成功，合入 {} 个远端提交，重放 {} 个本地提交",
                behind, ahead
            );
            return Ok(SelfUpdateResult {
                ok: true,
                pulled: behind,
                message: format!(
                    "检测到本地有 {} 个提交，已用 rebase 拉取 {} 个远端提交（本地提交已重放到最新之上）。源码已更新，开发模式将自动重新编译并重启以生效。",
                    ahead, behind
                ),
                restart_required: true,
            });
        }

        // Rebase hit a conflict (or otherwise failed): git leaves the repo
        // mid-rebase. Abort to restore the exact pre-pull state — `--autostash`
        // is re-applied on abort, so uncommitted edits are not lost.
        let (acode, _, aerr) = git
            .run(&["rebase", "--abort"])
            .await
            .unwrap_or((-1, String::new(), String::new()));
        warn!(
            "self_update_pull: rebase 拉取失败 (code={})，已 rebase --abort (code={}): {}{}",
            rcode,
            acode,
            rerr.trim(),
            if aerr.trim().is_empty() {
                String::new()
            } else {
                format!(" | abort: {}", aerr.trim())
            }
        );
        let detail: String = rerr.chars().take(500).collect();
        return Ok(SelfUpdateResult {
            ok: false,
            pulled: 0,
            message: format!(
                "本地与 origin/{} 已分叉，自动 rebase 拉取时发生冲突，已回滚到拉取前状态（你的改动未丢失）。请手动 rebase 解决冲突后重试。\n\n{}",
                project.branch_dev, detail
            ),
            restart_required: false,
        });
    }

    // ff-only failed without divergence (ahead==0): almost always uncommitted
    // changes that the update would overwrite. Surface git's own reason.
    let lowered = err.to_lowercase();
    let hint = if lowered.contains("local changes")
        || lowered.contains("would be overwritten")
        || lowered.contains("unstaged")
        || err.contains("覆盖")
        || err.contains("未提交")
    {
        "本地有未提交改动会被覆盖，已阻止拉取。请先提交或暂存(git stash)你的改动后重试，以免丢失。"
    } else {
        "拉取失败。"
    };

    Ok(SelfUpdateResult {
        ok: false,
        pulled: 0,
        message: format!("{}\n\n{}", hint, err.chars().take(500).collect::<String>()),
        restart_required: false,
    })
}
