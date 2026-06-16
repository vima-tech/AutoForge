use crate::core::git::GitProxy;
use crate::models::project::Project;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

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
async fn count_range(git: &GitProxy, range: &str) -> i64 {
    git.run(&["rev-list", "--count", range])
        .await
        .ok()
        .and_then(|(c, out, _)| (c == 0).then(|| out.trim().parse::<i64>().ok()).flatten())
        .unwrap_or(0)
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
        let _ = git.run(&["fetch", "origin", &p.branch_dev]).await;
        let behind = count_range(&git, &format!("HEAD..origin/{}", p.branch_dev)).await;
        return Ok(SelfUpdatePending {
            project_id: Some(p.id),
            behind,
        });
    }

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

    let branch = git
        .run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default();
    let is_self_managed = !branch.is_empty() && branch == project.branch_dev;

    let dirty = git
        .run(&["status", "--porcelain"])
        .await
        .map(|(_, out, _)| !out.trim().is_empty())
        .unwrap_or(false);

    // Best-effort refresh so behind/ahead reflect the real remote tip.
    let _ = git.run(&["fetch", "origin", &project.branch_dev]).await;
    let remote = format!("origin/{}", project.branch_dev);
    let behind = count_range(&git, &format!("HEAD..{}", remote)).await;
    let ahead = count_range(&git, &format!("{}..HEAD", remote)).await;

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

    let _ = git.run(&["fetch", "origin", &project.branch_dev]).await;
    let remote = format!("origin/{}", project.branch_dev);
    let behind = count_range(&git, &format!("HEAD..{}", remote)).await;
    if behind == 0 {
        return Ok(SelfUpdateResult {
            ok: true,
            pulled: 0,
            message: "已是最新，无需更新。".to_string(),
            restart_required: false,
        });
    }

    // Fast-forward only: never create a merge commit and never rewrite the
    // user's uncommitted edits. If local work conflicts, git refuses and we
    // surface that instead of risking data loss — the user commits/stashes first.
    let (code, _, err) = git
        .run(&["pull", "--ff-only", "origin", &project.branch_dev])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));

    if code == 0 {
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

    let lowered = err.to_lowercase();
    let hint = if lowered.contains("local changes")
        || lowered.contains("would be overwritten")
        || lowered.contains("unstaged")
    {
        "本地有未提交改动会被覆盖，已阻止拉取。请先提交或暂存(git stash)你的改动后重试，以免丢失。"
    } else if lowered.contains("non-fast-forward") || lowered.contains("diverge") {
        "本地与 origin/dev 已分叉，无法快进合并。请手动处理分叉后重试。"
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
