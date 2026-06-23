use crate::core::{event, git::GitProxy};
use crate::db::Db;
use crate::state::worktrees_base;
use anyhow::{anyhow, Result};
use tracing::info;

/// Revert a merged CR's squash commit on the integration (dev) branch.
///
/// Returns `Ok(())` once the revert commit has reached dev — committed in place
/// (normal projects) or pushed to `origin/<dev>` (self-managed) — or
/// `Err((exit_code, message))` describing why it didn't (most often: a later commit
/// depends on the reverted change, so `git revert` conflicts and is aborted).
///
/// Mirrors `merge::land_on_dev`'s dual path so撤销 reuses the exact same safe plumbing:
/// the in-place path never runs when dev is the live working tree (self-hosting), where
/// we instead revert inside a throwaway worktree and push to `origin/<dev>`.
async fn revert_on_dev(
    git: &GitProxy,
    project: &crate::models::project::Project,
    cr_id: &str,
    merge_commit: &str,
    dev_is_live: bool,
) -> std::result::Result<(), (i32, String)> {
    // AutoForge identity so `git revert --no-edit` can commit without a configured user.
    let revert_cmd: [&str; 6] = [
        "-c",
        "user.name=AutoForge",
        "-c",
        "user.email=autoforge@local",
        "revert",
        "--no-edit",
    ];

    if !dev_is_live {
        // Fast path (all non-self-managed projects): in-place checkout + revert.
        let (cc, _, ce) = git
            .run(&["checkout", &project.branch_dev])
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if cc != 0 {
            return Err((cc, format!("checkout {} 失败：{}", project.branch_dev, ce)));
        }
        let mut args = revert_cmd.to_vec();
        args.push(merge_commit);
        let (rc, _, re) = git
            .run(&args)
            .await
            .unwrap_or((-1, String::new(), "git not available".to_string()));
        if rc != 0 {
            // Conflict (or bad ref): abort so dev stays clean for a manual fix.
            let _ = git.run(&["revert", "--abort"]).await;
            return Err((rc, re));
        }
        return Ok(());
    }

    // Isolated path (self-managed: dev is checked out live). Revert in a throwaway
    // worktree based on the newest origin/<dev> and push, never touching the live tree.
    let _ = git.run(&["fetch", "origin", &project.branch_dev]).await;
    let remote_ref = format!("origin/{}", project.branch_dev);
    let base = if git
        .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false)
    {
        remote_ref
    } else {
        project.branch_dev.clone()
    };

    let tmp_branch = format!("autoforge/revert-{}", cr_id);
    let tmp_path = format!("{}/.revert-{}", worktrees_base(), cr_id);
    // Clear any stale worktree/branch left by a previous failed attempt.
    let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
    let _ = git.run(&["worktree", "prune"]).await;
    let _ = git.run(&["branch", "-D", &tmp_branch]).await;

    let (wc, _, we) = git
        .run(&["worktree", "add", "-b", &tmp_branch, &tmp_path, &base])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if wc != 0 {
        return Err((wc, format!("创建隔离撤销 worktree 失败：{}", we)));
    }

    let tmp_git = GitProxy::new(&tmp_path);
    let mut args = revert_cmd.to_vec();
    args.push(merge_commit);
    let (rc, _, re) = tmp_git
        .run(&args)
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if rc != 0 {
        let _ = tmp_git.run(&["revert", "--abort"]).await;
        let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
        let _ = git.run(&["branch", "-D", &tmp_branch]).await;
        return Err((rc, re));
    }

    let push_target = format!("HEAD:{}", project.branch_dev);
    let (pc, _, pe) = tmp_git
        .run(&["push", "origin", &push_target])
        .await
        .unwrap_or((-1, String::new(), "git not available".to_string()));
    if pc != 0 {
        let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
        return Err((
            pc,
            format!(
                "撤销提交已生成但推送 origin/{} 失败（请检查 SSH/网络）。改动保留在本地分支 `{}`，可手动恢复。\n{}",
                project.branch_dev, tmp_branch, pe
            ),
        ));
    }

    let _ = git.run(&["worktree", "remove", "--force", &tmp_path]).await;
    let _ = git.run(&["branch", "-D", &tmp_branch]).await;
    Ok(())
}

/// Revert job entry point: undo a merged CR's changes on dev.
///
/// Precondition: the command `revert_change_request` has atomically flipped the CR to
/// `reverting` (a `merged`-only gate), so a double click can't enqueue twice. On success
/// the CR/issue become `reverted`; on conflict/failure they are restored to `merged` and
/// the reason is surfaced for manual handling (撤不干净 = a later change depends on it).
pub async fn run(db: &Db, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("no worktree session for cr {}", cr_id))?;
    let cr = sqlx::query_as::<_, crate::models::change_request::ChangeRequest>(
        "SELECT * FROM change_requests WHERE id=?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("cr {} not found", cr_id))?;
    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&cr.project_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("project {} not found", cr.project_id))?;

    let merge_commit = match session.merge_commit.as_deref().map(str::trim) {
        Some(sha) if !sha.is_empty() => sha.to_string(),
        _ => {
            // No recorded commit (merged before this feature / empty CR) → restore + report.
            restore_merged(db, app, cr_id, &cr.issue_id, "该需求无可撤销的合并提交记录（合并早于撤销功能或为空改动）").await;
            return Ok(());
        }
    };

    // Serialize with merge/resolve for this project + CR (order: merge_lock → cr_lock).
    let merge_lock = crate::state::merge_lock(&cr.project_id);
    let _merge_guard = merge_lock.lock().await;
    let cr_lock = crate::state::cr_lock(cr_id);
    let _cr_guard = cr_lock.lock().await;

    let git = GitProxy::new(&project.repo_path);
    let live_branch = git
        .run(&["branch", "--show-current"])
        .await
        .ok()
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_default();
    let dev_is_live = !live_branch.is_empty() && live_branch == project.branch_dev;

    event::emit(
        app,
        event::AppEvent::TaskProgress {
            cr_id: cr_id.to_string(),
            phase: "reverting".to_string(),
            note: Some(format!("正在撤销该需求改动（revert {}）…", &merge_commit[..merge_commit.len().min(8)])),
        },
    );

    match revert_on_dev(&git, &project, cr_id, &merge_commit, dev_is_live).await {
        Ok(()) => {
            sqlx::query("UPDATE change_requests SET status='reverted', updated_at=datetime('now') WHERE id=?")
                .bind(cr_id)
                .execute(db)
                .await?;
            // 合并 CR：撤销整条 squash 提交即撤销全部成员需求 → 全组置 reverted。
            crate::commands::change_requests::set_cr_issues_status(db, cr_id, "reverted").await?;
            crate::core::notify::dispatch(db, "cr_reverted", "需求改动已撤销", cr_id).await;
            event::emit(
                app,
                event::AppEvent::CrReverted {
                    cr_id: cr_id.to_string(),
                    project_id: cr.project_id.clone(),
                },
            );
            info!("revert completed for cr {} (revert of {})", cr_id, merge_commit);
            Ok(())
        }
        Err((code, err)) => {
            info!("revert failed for cr {} ({}): {}", cr_id, code, err);
            let reason = format!(
                "## 撤销失败\n\n无法在 `{}` 上撤销该需求的合并提交 `{}`（退出码 {}）。\n\n常见原因：**后续需求改动依赖了被撤销的代码**，导致 revert 冲突——这是「撤不干净」，需人工处理。\n\n```\n{}\n```\n",
                project.branch_dev,
                merge_commit,
                code,
                err.chars().take(2000).collect::<String>()
            );
            let _ = sqlx::query("UPDATE worktree_sessions SET report_content=? WHERE id=?")
                .bind(&reason)
                .bind(&session.id)
                .execute(db)
                .await;
            restore_merged(db, app, cr_id, &cr.issue_id, &err.chars().take(300).collect::<String>()).await;
            Ok(())
        }
    }
}

/// Restore a CR/issue to `merged` after a revert that didn't land, and surface why.
async fn restore_merged(db: &Db, app: &tauri::AppHandle, cr_id: &str, issue_id: &str, msg: &str) {
    let _ = sqlx::query("UPDATE change_requests SET status='merged', updated_at=datetime('now') WHERE id=?")
        .bind(cr_id)
        .execute(db)
        .await;
    // 合并 CR：全部成员需求一并回到 merged（revert 没落地）。
    let _ = crate::commands::change_requests::set_cr_issues_status(db, cr_id, "merged").await;
    let _ = issue_id; // 旧单需求参数保留以兼容签名；真源走关联表。
    event::emit(
        app,
        event::AppEvent::WorktreeUpdate {
            cr_id: cr_id.to_string(),
            status: "merged".to_string(),
            message: Some(format!("撤销未完成：{}", msg)),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn rev_parse(dir: &Path, r: &str) -> String {
        let out = Command::new("git")
            .args(["rev-parse", r])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn proj(repo: &str) -> crate::models::project::Project {
        crate::models::project::Project {
            id: "p1".into(),
            name: "p".into(),
            slug: "p".into(),
            description: String::new(),
            repo_path: repo.into(),
            branch_dev: "dev".into(),
            branch_main: "main".into(),
            status: "active".into(),
            config_yaml: None,
            is_default: false,
            archived_at: None,
            code_agent_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// In-place revert (dev not live): the reverted change is gone, dev advances by one
    /// revert commit, and no conflict occurs for an independent later commit.
    #[tokio::test]
    async fn revert_on_dev_undoes_squash_commit() {
        let base = std::env::temp_dir().join(format!(
            "af-revert-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "tester"]);
        std::fs::write(repo.join("f.txt"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["branch", "dev"]);
        git(&repo, &["checkout", "-q", "dev"]);
        // The CR's squash commit on dev: adds feature.txt.
        std::fs::write(repo.join("feature.txt"), "feature\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "feat: add feature"]);
        let sha = rev_parse(&repo, "HEAD");
        // An independent later commit (must NOT block the revert).
        std::fs::write(repo.join("other.txt"), "other\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "chore: other"]);
        // Leave the main tree on a non-dev branch so dev_is_live=false path runs.
        git(&repo, &["checkout", "-q", "main"]);

        let gp = GitProxy::new(repo.to_str().unwrap());
        let p = proj(repo.to_str().unwrap());
        let res = revert_on_dev(&gp, &p, "cr1", &sha, false).await;
        assert!(res.is_ok(), "revert should succeed: {res:?}");

        // feature.txt is gone on dev; other.txt remains.
        git(&repo, &["checkout", "-q", "dev"]);
        assert!(!repo.join("feature.txt").exists(), "reverted file should be gone");
        assert!(repo.join("other.txt").exists(), "later independent change must remain");

        let _ = std::fs::remove_dir_all(base);
    }

    /// Revert conflict (a later commit depends on the reverted change) aborts cleanly:
    /// dev is left untouched (no half-applied revert, no conflict markers).
    #[tokio::test]
    async fn revert_conflict_aborts_and_leaves_dev_clean() {
        let base = std::env::temp_dir().join(format!(
            "af-revert-conflict-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "tester"]);
        std::fs::write(repo.join("f.txt"), "l1\nl2\nl3\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["branch", "dev"]);
        git(&repo, &["checkout", "-q", "dev"]);
        // CR commit edits l2.
        std::fs::write(repo.join("f.txt"), "l1\nCR\nl3\n").unwrap();
        git(&repo, &["commit", "-qam", "feat: edit l2"]);
        let sha = rev_parse(&repo, "HEAD");
        // A later commit further edits the SAME line → revert of the CR conflicts.
        std::fs::write(repo.join("f.txt"), "l1\nLATER\nl3\n").unwrap();
        git(&repo, &["commit", "-qam", "chore: edit l2 again"]);
        let dev_tip = rev_parse(&repo, "dev");
        git(&repo, &["checkout", "-q", "main"]);

        let gp = GitProxy::new(repo.to_str().unwrap());
        let p = proj(repo.to_str().unwrap());
        let res = revert_on_dev(&gp, &p, "cr1", &sha, false).await;
        assert!(res.is_err(), "revert should conflict and error");

        // dev tip unchanged (revert was aborted, no commit, no markers).
        assert_eq!(rev_parse(&repo, "dev"), dev_tip, "dev must be untouched after abort");
        git(&repo, &["checkout", "-q", "dev"]);
        let content = std::fs::read_to_string(repo.join("f.txt")).unwrap();
        assert!(!content.contains("<<<<<<<"), "no conflict markers should remain: {content}");

        let _ = std::fs::remove_dir_all(base);
    }
}
