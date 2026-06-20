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

/// Fetch `origin/<branch>` and log the outcome; returns whether it succeeded.
/// The callers previously discarded this result (`let _ = ...`), which hid
/// network/SSH failures that leave behind/ahead stale — exactly the silent
/// failure mode we need visibility into. `self_update_pull` now refuses to act
/// when this returns false, to never update off a stale remote ref.
async fn fetch_and_log(git: &GitProxy, branch: &str, ctx: &str) -> bool {
    match git.run(&["fetch", "origin", branch]).await {
        Ok((0, _, _)) => {
            info!("self_update[{}]: fetch origin/{} 成功", ctx, branch);
            true
        }
        Ok((code, _, err)) => {
            warn!(
                "self_update[{}]: fetch origin/{} 失败 (code={}): {}",
                ctx,
                branch,
                code,
                err.trim()
            );
            false
        }
        Err(e) => {
            warn!("self_update[{}]: fetch origin/{} 执行错误: {}", ctx, branch, e);
            false
        }
    }
}

/// Current branch name (empty string on detached HEAD or error).
async fn current_branch(git: &GitProxy) -> String {
    git.run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default()
}

/// Files currently unmerged (in conflict) in the working tree.
async fn conflicted_files(git: &GitProxy) -> Vec<String> {
    match git.run(&["diff", "--name-only", "--diff-filter=U"]).await {
        Ok((0, out, _)) => out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
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
    // Serialize against concurrent merge tasks operating on the same live dev
    // tree: tasks/merge.rs holds this exact per-project lock during its
    // `checkout dev && merge`. Without it, a merge landing while we rebase the
    // working tree is a race over the same files — the multi-mode hazard.
    let lock = crate::state::merge_lock(&project.id);
    let _guard = lock.lock().await;
    let git = GitProxy::new(&project.repo_path);
    Ok(run_self_update_pull(&git, &project).await)
}

/// Core of the self-update pull, free of Tauri types (caller holds the merge
/// lock). Hardened for multi-developer / mixed-mode repos where the live `dev`
/// routinely carries local commits and uncommitted edits:
///
/// 1. refuse unless actually on `branch_dev` (never rewrite the wrong history);
/// 2. refuse if the fetch failed (never act on a stale remote ref);
/// 3. record a recovery branch before any rewrite;
/// 4. clean repo → fast-forward; diverged repo → explicit stash + rerere-aided
///    rebase, every failure rolled back so the repo is never left broken.
async fn run_self_update_pull(git: &GitProxy, project: &Project) -> SelfUpdateResult {
    let branch_dev = &project.branch_dev;
    let fail = |msg: String| SelfUpdateResult {
        ok: false,
        pulled: 0,
        message: msg,
        restart_required: false,
    };
    info!(
        "self_update_pull: 项目 {} repo={} 开始",
        project.id, project.repo_path
    );

    // 1) Precondition: must be on the dev branch.
    let branch = current_branch(git).await;
    if branch.is_empty() || &branch != branch_dev {
        let cur = if branch.is_empty() {
            "游离 HEAD".to_string()
        } else {
            branch.clone()
        };
        warn!(
            "self_update_pull: 当前分支={} 非 {}，拒绝自动更新",
            cur, branch_dev
        );
        return fail(format!(
            "当前不在 {} 分支（现在是 {}），为避免改错历史已停止。请先切回 {} 分支再同步。",
            branch_dev, cur, branch_dev
        ));
    }

    // 2) Fetch must succeed — never update off a stale remote ref.
    if !fetch_and_log(git, branch_dev, "pull").await {
        return fail(format!(
            "无法拉取 origin/{}（网络或凭据问题），已停止以免基于过期数据更新。请检查网络/SSH 后重试。",
            branch_dev
        ));
    }

    let remote = format!("origin/{}", branch_dev);
    let behind = count_range(git, &format!("HEAD..{}", remote)).await;
    let ahead = count_range(git, &format!("{}..HEAD", remote)).await;
    info!("self_update_pull: behind={} ahead={}", behind, ahead);
    if behind == 0 {
        info!("self_update_pull: 已是最新，跳过");
        return SelfUpdateResult {
            ok: true,
            pulled: 0,
            message: "已是最新，无需更新。".to_string(),
            restart_required: false,
        };
    }

    // 3) Recovery point before any history rewrite. `autoforge/*` is allowed by
    //    GitProxy and deletable later; -f overwrites any prior backup.
    let head = git
        .run(&["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|(_, o, _)| o.trim().to_string())
        .unwrap_or_default();
    let backup_ref = "autoforge/self-update-backup";
    let _ = git.run(&["branch", "-f", backup_ref, "HEAD"]).await;
    info!("self_update_pull: 记录恢复点 {} -> {}", backup_ref, head);

    // 4a) No local commits → plain fast-forward (zero risk of rewriting).
    if ahead == 0 {
        let (code, _, err) = git
            .run(&["pull", "--ff-only", "origin", branch_dev])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        info!("self_update_pull: ff-only 退出码={}", code);
        if code == 0 {
            info!("self_update_pull: 快进 {} 个提交成功", behind);
            return SelfUpdateResult {
                ok: true,
                pulled: behind,
                message: format!(
                    "已拉取 {} 个提交。源码已更新，开发模式将自动重新编译并重启以生效。",
                    behind
                ),
                restart_required: true,
            };
        }
        warn!("self_update_pull: ff-only 失败 (code={}): {}", code, err.trim());
        // ahead==0 yet ff failed → uncommitted changes would be overwritten.
        let detail: String = err.chars().take(400).collect();
        return fail(format!(
            "本地有未提交改动会被覆盖，已阻止拉取。请先提交或暂存(git stash)后重试。\n\n{}",
            detail
        ));
    }

    // 4b) Diverged (ahead>0): replay local commits onto the new remote tip via
    //     an explicit, fully recoverable rebase.
    info!("self_update_pull: 检测到分叉(本地领先 {})，改用 rebase", ahead);

    let dirty = git
        .run(&["status", "--porcelain"])
        .await
        .map(|(_, o, _)| !o.trim().is_empty())
        .unwrap_or(false);
    let mut stashed = false;
    if dirty {
        let (sc, _, serr) = git
            .run(&["stash", "push", "-u", "-m", "autoforge-self-update"])
            .await
            .unwrap_or((-1, String::new(), String::new()));
        if sc != 0 {
            warn!("self_update_pull: stash 暂存失败 (code={}): {}", sc, serr.trim());
            return fail(format!(
                "暂存未提交改动失败，已停止以免丢失。请手动处理后重试。\n\n{}",
                serr.chars().take(300).collect::<String>()
            ));
        }
        stashed = true;
        info!("self_update_pull: 已暂存未提交改动");
    }

    // Reuse remembered conflict resolutions (consistent with tasks/merge.rs).
    let _ = git.run(&["config", "rerere.enabled", "true"]).await;

    let (rc, _, rerr) = git
        .run(&["rebase", &remote])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    info!("self_update_pull: rebase {} 退出码={}", remote, rc);

    if rc != 0 {
        // Conflict rerere could not auto-resolve → roll back fully (chosen
        // policy): never leave the repo mid-rebase.
        let files = conflicted_files(git).await;
        let _ = git.run(&["rebase", "--abort"]).await;
        if stashed {
            let _ = git.run(&["stash", "pop"]).await;
        }
        warn!(
            "self_update_pull: rebase 冲突已回滚，冲突文件={:?}: {}",
            files,
            rerr.trim()
        );
        let flist = if files.is_empty() {
            String::new()
        } else {
            format!("\n\n冲突文件：\n{}", files.join("\n"))
        };
        return fail(format!(
            "与 origin/{} 存在无法自动合并的冲突，已回滚到更新前状态（你的提交与未提交改动均未丢失，恢复点分支 {}）。请手动 rebase 解决冲突后重试。{}",
            branch_dev, backup_ref, flist
        ));
    }

    // Rebase succeeded — restore the shelved edits.
    if stashed {
        let (pc, _, perr) = git
            .run(&["stash", "pop"])
            .await
            .unwrap_or((-1, String::new(), String::new()));
        if pc != 0 {
            // Code is updated, but the user's uncommitted edits collide with the
            // pulled changes. `stash pop` keeps the stash on conflict; reset the
            // tree clean to the new HEAD and leave the edits safely stashed.
            let files = conflicted_files(git).await;
            let _ = git.run(&["reset", "--hard", "HEAD"]).await;
            warn!(
                "self_update_pull: stash pop 冲突，改动保留在 stash，文件={:?}: {}",
                files,
                perr.trim()
            );
            let flist = if files.is_empty() {
                String::new()
            } else {
                format!("\n\n涉及文件：\n{}", files.join("\n"))
            };
            return SelfUpdateResult {
                ok: true,
                pulled: behind,
                message: format!(
                    "源码已更新到最新（合入 {} 个远端提交），但你之前未提交的改动与更新有冲突，已原样保留在 git stash（stash@{{0}}）。请手动 `git stash pop` 解决冲突。开发模式会自动重新编译并重启。{}",
                    behind, flist
                ),
                restart_required: true,
            };
        }
        info!("self_update_pull: 已恢复暂存改动");
    }

    info!(
        "self_update_pull: rebase 成功，合入 {} 远端提交，重放 {} 本地提交",
        behind, ahead
    );
    SelfUpdateResult {
        ok: true,
        pulled: behind,
        message: format!(
            "检测到本地有 {} 个提交，已用 rebase 拉取 {} 个远端提交（本地提交重放到最新之上，未提交改动已恢复）。源码已更新，开发模式将自动重新编译并重启以生效。",
            ahead, behind
        ),
        restart_required: true,
    }
}
