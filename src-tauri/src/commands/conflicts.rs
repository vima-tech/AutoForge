//! 人工解决合并冲突（方案 B 逐 hunk 决策 + 方案 C 外部 IDE 兜底）。
//!
//! 冲突现场按需「物化」到 CR worktree（`merge::materialize_conflict`，幂等），三方真源来自
//! 工作区里带 `<<<<<<< ======= >>>>>>>` 标记的文件内容（解析成有序段供前端渲染）。AI 自动
//! 解、应用内决策、外部 IDE 解决三路共享同一条 `merge::finalize_resolution` 收尾（提交→测试门
//! →回代码审核），绝不直接落 dev。所有写 worktree 的操作持 `state::cr_lock` 串行（H2）。

use crate::core::git::GitProxy;
use crate::models::change_request::ChangeRequest;
use crate::models::issue::Issue;
use crate::models::project::Project;
use crate::models::worktree::WorktreeSession;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, State};

/// 冲突文件里的一个有序片段：上下文（context）或一处冲突（conflict，含两侧/基线）。
#[derive(Serialize)]
pub struct ConflictSegment {
    pub kind: &'static str, // "context" | "conflict"
    /// context 片段的原文。
    pub text: Option<String>,
    /// conflict：本分支（ours / HEAD）一侧。
    pub ours: Option<String>,
    /// conflict：dev（theirs）一侧。
    pub theirs: Option<String>,
    /// conflict：共同祖先（diff3 风格才有，普通合并为空）。
    pub base: Option<String>,
}

/// 一个冲突文件的结构化现场。
#[derive(Serialize)]
pub struct ConflictFile {
    pub path: String,
    /// 二进制 / 非 UTF-8 文件无法逐 hunk 渲染，前端引导走外部 IDE。
    pub binary: bool,
    pub segments: Vec<ConflictSegment>,
}

/// 整个 CR 的冲突现场。
#[derive(Serialize)]
pub struct ConflictDetail {
    pub files: Vec<ConflictFile>,
    /// 物化后是否仍有需人工决策的冲突（false=rerere/自动已解，可直接提交复审）。
    pub resolvable: bool,
}

/// 把带冲突标记的文件内容解析成有序片段。普通合并标记：
/// `<<<<<<< ours` … `=======` … `>>>>>>> theirs`；diff3 多一段 `||||||| base`。
fn parse_conflict_segments(content: &str) -> Vec<ConflictSegment> {
    let mut segs: Vec<ConflictSegment> = Vec::new();
    let mut ctx: Vec<&str> = Vec::new();
    let flush_ctx = |ctx: &mut Vec<&str>, segs: &mut Vec<ConflictSegment>| {
        if !ctx.is_empty() {
            segs.push(ConflictSegment {
                kind: "context",
                text: Some(ctx.join("\n")),
                ours: None,
                theirs: None,
                base: None,
            });
            ctx.clear();
        }
    };

    let lines: Vec<&str> = content.split('\n').collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("<<<<<<<") {
            flush_ctx(&mut ctx, &mut segs);
            let mut ours: Vec<&str> = Vec::new();
            let mut base: Vec<&str> = Vec::new();
            let mut theirs: Vec<&str> = Vec::new();
            let mut phase = 0u8; // 0=ours, 1=base(diff3), 2=theirs
            i += 1;
            while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                let l = lines[i];
                if l.starts_with("|||||||") {
                    phase = 1;
                } else if l.starts_with("=======") {
                    phase = 2;
                } else {
                    match phase {
                        0 => ours.push(l),
                        1 => base.push(l),
                        _ => theirs.push(l),
                    }
                }
                i += 1;
            }
            // skip the closing >>>>>>> marker
            segs.push(ConflictSegment {
                kind: "conflict",
                text: None,
                ours: Some(ours.join("\n")),
                theirs: Some(theirs.join("\n")),
                base: if base.is_empty() { None } else { Some(base.join("\n")) },
            });
        } else {
            ctx.push(line);
        }
        i += 1;
    }
    flush_ctx(&mut ctx, &mut segs);
    segs
}

/// 载入最新 session + cr + project + issue（四件套，供解冲突各命令复用）。
async fn load_ctx(
    db: &crate::db::Db,
    cr_id: &str,
) -> Result<(WorktreeSession, ChangeRequest, Project, Issue), String> {
    let session = sqlx::query_as::<_, WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("无 worktree 会话：{}", cr_id))?;
    let cr = sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
        .bind(cr_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("变更请求不存在：{}", cr_id))?;
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&cr.project_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("项目不存在：{}", cr.project_id))?;
    let issue = sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&cr.issue_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("需求不存在：{}", cr.issue_id))?;
    Ok((session, cr, project, issue))
}

/// 读取某 CR 的冲突现场（三方有序段），按需物化冲突态。供应用内逐 hunk 解决器渲染。
#[tauri::command]
pub async fn get_conflict_detail(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<ConflictDetail, String> {
    let (session, cr, project, _issue) = load_ctx(&state.db, &cr_id).await?;
    // Only a CR actually in conflict has a现场 to materialize; guard so we never run a
    // `git merge` (materialize side-effect) on a non-conflict CR's worktree by mistake.
    if cr.status != "merge_conflict" {
        return Err(format!("该变更当前状态为 {}，无冲突现场", cr.status));
    }
    if !std::path::Path::new(&session.worktree_path).exists() {
        return Err("CR worktree 不存在，无法读取冲突现场".to_string());
    }
    // Serialize with merge/resolve so we never materialize underneath another writer.
    let cr_lock = crate::state::cr_lock(&cr_id);
    let _guard = cr_lock.lock().await;

    let wt = GitProxy::new(&session.worktree_path);
    let _ = crate::tasks::merge::materialize_conflict(&wt, &session.branch_name, &project.branch_dev)
        .await;
    let unmerged = crate::tasks::merge::list_unmerged(&wt).await;

    let mut files = Vec::new();
    for path in unmerged {
        let abs = std::path::Path::new(&session.worktree_path).join(&path);
        match std::fs::read_to_string(&abs) {
            Ok(text) => files.push(ConflictFile {
                segments: parse_conflict_segments(&text),
                path,
                binary: false,
            }),
            Err(_) => files.push(ConflictFile {
                path,
                binary: true,
                segments: Vec::new(),
            }),
        }
    }
    Ok(ConflictDetail {
        resolvable: !files.is_empty(),
        files,
    })
}

/// worktree 内的相对路径是否安全（不越界到 worktree 之外）。
fn path_within(worktree: &str, rel: &str) -> bool {
    let root = match std::fs::canonicalize(worktree) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // 目标父目录可能还不存在（一般冲突文件已存在），逐级规整：先 join 再 canonicalize 父目录。
    let target = root.join(rel);
    match target.parent().and_then(|p| std::fs::canonicalize(p).ok()) {
        Some(parent) => parent.starts_with(&root),
        None => false,
    }
}

/// 人工解决合并冲突并复审。
///
/// `files=Some(path→content)`：应用内逐 hunk 决策器拼好的整文件内容，先写盘；`files=None`：
/// 外部 IDE 已在 worktree 改好盘，直接收尾。两路都走 `finalize_resolution`（提交→测试门→回
/// 代码审核）。长任务，spawn 到后台，命令立即返回。
#[tauri::command]
pub async fn resolve_conflict_manually(
    cr_id: String,
    files: Option<HashMap<String, String>>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 预校验：必须处于 merge_conflict（避免对非冲突 CR 误触发）。
    let status: Option<String> =
        sqlx::query_as::<_, (String,)>("SELECT status FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .map(|(s,)| s);
    if status.as_deref() != Some("merge_conflict") {
        return Err(format!(
            "该变更当前状态为 {}，不可解冲突",
            status.unwrap_or_else(|| "未知".into())
        ));
    }

    let db = state.db.clone();
    let tx = state.job_tx.clone();
    tokio::spawn(async move {
        // Hold the CR lock for the whole resolve (H2); re-check status under it (dedup).
        let cr_lock = crate::state::cr_lock(&cr_id);
        let _guard = cr_lock.lock().await;
        let cur = sqlx::query_as::<_, (String,)>("SELECT status FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_optional(&db)
            .await
            .ok()
            .flatten()
            .map(|(s,)| s);
        if cur.as_deref() != Some("merge_conflict") {
            tracing::info!("resolve_conflict_manually: cr {} not in conflict ({:?}); skip", cr_id, cur);
            return;
        }
        let (session, cr, project, issue) = match load_ctx(&db, &cr_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("resolve_conflict_manually load_ctx failed for {}: {}", cr_id, e);
                return;
            }
        };
        let wt = GitProxy::new(&session.worktree_path);
        // Ensure the conflict is materialized (mid-merge) so the commit completes the merge.
        let dev_ref =
            crate::tasks::merge::materialize_conflict(&wt, &session.branch_name, &project.branch_dev)
                .await;

        // Apply the in-app resolutions (full resolved file contents). External-IDE path
        // (files=None) skips this — the user already edited the worktree on disk.
        if let Some(files) = files {
            for (rel, content) in files {
                if !path_within(&session.worktree_path, &rel) {
                    tracing::warn!("resolve_conflict_manually: rejecting out-of-tree path {}", rel);
                    continue;
                }
                let abs = std::path::Path::new(&session.worktree_path).join(&rel);
                if let Err(e) = std::fs::write(&abs, content) {
                    tracing::warn!("resolve_conflict_manually: write {} failed: {}", rel, e);
                }
            }
        }

        let commit_msg = format!(
            "AutoForge: 人工解决合并冲突（{} → {}）",
            dev_ref, session.branch_name
        );
        if let Err(e) =
            crate::tasks::merge::finalize_resolution(&db, &tx, &app, &session, &cr, &issue, &commit_msg)
                .await
        {
            tracing::warn!("resolve_conflict_manually finalize failed for {}: {}", cr_id, e);
        }
    });
    Ok(())
}

/// 外部 IDE 兜底：物化冲突态，并在系统文件管理器中定位 CR worktree，供用户用自己的 IDE 解决。
/// 解完回应用点「我已在外部解决·复审」(`resolve_conflict_manually(files=None)`)。
#[tauri::command]
pub async fn open_conflict_workspace(
    cr_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let (session, cr, project, _issue) = load_ctx(&state.db, &cr_id).await?;
    if cr.status != "merge_conflict" {
        return Err(format!("该变更当前状态为 {}，无冲突现场", cr.status));
    }
    if !std::path::Path::new(&session.worktree_path).exists() {
        return Err("CR worktree 不存在".to_string());
    }
    {
        let cr_lock = crate::state::cr_lock(&cr_id);
        let _guard = cr_lock.lock().await;
        let wt = GitProxy::new(&session.worktree_path);
        let _ =
            crate::tasks::merge::materialize_conflict(&wt, &session.branch_name, &project.branch_dev)
                .await;
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .reveal_item_in_dir(&session.worktree_path)
        .map_err(|e| e.to_string())?;
    Ok(session.worktree_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_two_way_conflict() {
        let content = "ctx-before\n<<<<<<< HEAD\nours-line\n=======\ntheirs-line\n>>>>>>> dev\nctx-after";
        let segs = parse_conflict_segments(content);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].kind, "context");
        assert_eq!(segs[0].text.as_deref(), Some("ctx-before"));
        assert_eq!(segs[1].kind, "conflict");
        assert_eq!(segs[1].ours.as_deref(), Some("ours-line"));
        assert_eq!(segs[1].theirs.as_deref(), Some("theirs-line"));
        assert!(segs[1].base.is_none());
        assert_eq!(segs[2].kind, "context");
        assert_eq!(segs[2].text.as_deref(), Some("ctx-after"));
    }

    #[test]
    fn parses_diff3_with_base_section() {
        let content = "<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> dev";
        let segs = parse_conflict_segments(content);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].ours.as_deref(), Some("ours"));
        assert_eq!(segs[0].base.as_deref(), Some("orig"));
        assert_eq!(segs[0].theirs.as_deref(), Some("theirs"));
    }

    #[test]
    fn no_markers_is_all_context() {
        let segs = parse_conflict_segments("line1\nline2\nline3");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].kind, "context");
    }
}
