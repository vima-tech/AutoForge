//! 活动通知收件箱命令。
//!
//! 收件箱把后端广播的 `AppEvent` 中「有动作价值」的流水线事件沉淀下来
//! （筛选逻辑见 `models::notification::NotificationDraft::from_event`），
//! 供前端 rail-me 活动中心回看、标记已读、跳转对应页面。
//!
//! 写入入口（`record_notification`）是不带 Tauri 类型的纯 async fn，由
//! `core::event::emit` 这一传输适配层在广播之后顺手调用（遵守 CLAUDE.md 铁律 #1/#3）。

use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::core::event::AppEvent;
use crate::db::Db;
use crate::models::notification::{Notification, NotificationDraft};
use crate::state::AppState;

/// 收件箱保留上限：仅留最近 N 条，避免无限增长。
const RETENTION_LIMIT: i64 = 500;

/// 把一个事件（若值得沉淀）写入收件箱并做保留裁剪。纯 async fn，无 Tauri 依赖。
///
/// 非通知类事件（心跳/高频/已有角标覆盖）返回 `Ok(())` 直接跳过。
pub async fn record_notification(db: &Db, ev: &AppEvent) -> Result<(), String> {
    let Some(mut draft) = NotificationDraft::from_event(ev) else {
        return Ok(());
    };
    // 把 CR 维度的阶段事件归并到所属需求(issue)线程：from_event 先以 cr_id 占位，
    // 这里查 change_requests 解析回 issue_id，让一条需求的录入/分析/审核/测试/合并/审计
    // 全部落到同一 thread_key 上，UPSERT 后只占收件箱一行。解析失败则保留 cr_id 兜底
    // （至少同一 CR 的各阶段仍折叠为一行）。
    if let Some(cr_id) = ev.cr_id() {
        if let Some(issue_id) = lookup_issue_id(db, cr_id).await? {
            draft.thread_key = Some(issue_id);
        }
    }
    upsert_draft(db, &draft).await?;
    prune(db).await
}

/// 由变更请求 id 反查所属需求 id；找不到返回 None。
async fn lookup_issue_id(db: &Db, cr_id: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT issue_id FROM change_requests WHERE id = ?")
        .bind(cr_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())
}

/// 按需求线程 UPSERT：同一 thread_key 的通知就地刷新（标题/内容/分类/跳转随最新阶段更新，
/// 并重置为未读、置顶时间），而非新插一行。无线程键时退化为普通插入。
async fn upsert_draft(db: &Db, draft: &NotificationDraft) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notifications
            (id, category, kind, title, body, link_page, link_ref, thread_key, read, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, datetime('now'))
         ON CONFLICT(thread_key) DO UPDATE SET
            category   = excluded.category,
            kind       = excluded.kind,
            title      = excluded.title,
            body       = excluded.body,
            link_page  = excluded.link_page,
            link_ref   = excluded.link_ref,
            read       = 0,
            created_at = datetime('now')",
    )
    .bind(&id)
    .bind(draft.category)
    .bind(draft.kind)
    .bind(&draft.title)
    .bind(&draft.body)
    .bind(draft.link_page)
    .bind(draft.link_ref.as_deref())
    .bind(draft.thread_key.as_deref())
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn prune(db: &Db) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM notifications
         WHERE id NOT IN (
             SELECT id FROM notifications ORDER BY created_at DESC, id DESC LIMIT ?
         )",
    )
    .bind(RETENTION_LIMIT)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 失效自愈：把「已不再需要操作者动作」的未读提醒类通知就地标为已读，
/// 停止角标/收件箱的持续提醒，但保留历史行供回看（不删除）。
///
/// 之所以需要这一步：审核/迭代等「需介入」通知由 `ReviewNeeded`/`IterationWarning`
/// 事件产生，而操作者真正完成审核走的是 `review_1`/`review_2` 命令——这些命令推进了
/// issue/CR 状态，却未必再发一个落到同一 `thread_key` 的事件来覆盖原通知，于是原提醒
/// 会一直停在未读。这里在每次拉取收件箱时按实体当前状态对账，自动消除失效提醒。
///
/// 仅触碰「会持续提醒」的 kind（analysis_completed / review_needed / iteration_warning）；
/// result/intake 类是终态事实，不在此清理。全部为集合式 UPDATE，开销极小。
async fn reconcile_stale(db: &Db) -> Result<(), String> {
    // 1) 「分析完成·待审核 1」：issue 已离开 pending_review_1（已审核1或被拒）即失效。
    sqlx::query(
        "UPDATE notifications SET read = 1
         WHERE read = 0 AND kind = 'analysis_completed'
           AND link_ref IS NOT NULL AND link_ref <> ''
           AND link_ref NOT IN (SELECT id FROM issues WHERE status = 'pending_review_1')",
    )
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 2) 「需要审核·节点 2」：绑定了具体 CR，CR 不再处于 pending_review_2 即失效。
    sqlx::query(
        "UPDATE notifications SET read = 1
         WHERE read = 0 AND kind = 'review_needed'
           AND link_ref IN (SELECT id FROM change_requests)
           AND link_ref NOT IN (SELECT id FROM change_requests WHERE status = 'pending_review_2')",
    )
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 3) 「需要审核·节点 1」：无具体 CR（link_ref 空/非 CR，stage 1 折叠成单行），
    //    全局已无任何待审核 1 的 issue 即失效。
    sqlx::query(
        "UPDATE notifications SET read = 1
         WHERE read = 0 AND kind = 'review_needed'
           AND (link_ref IS NULL OR link_ref = '' OR link_ref NOT IN (SELECT id FROM change_requests))
           AND NOT EXISTS (SELECT 1 FROM issues WHERE status = 'pending_review_1')",
    )
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    // 4) 「迭代次数告警」：CR 已离开执行阶段（不再迭代）即失效。
    sqlx::query(
        "UPDATE notifications SET read = 1
         WHERE read = 0 AND kind = 'iteration_warning'
           AND link_ref IN (SELECT id FROM change_requests)
           AND link_ref NOT IN (
               SELECT id FROM change_requests WHERE status IN ('executing', 'pending_execution')
           )",
    )
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

async fn fetch_recent(db: &Db, limit: i64) -> Result<Vec<Notification>, String> {
    sqlx::query_as::<_, Notification>(
        "SELECT id, category, kind, title, body, link_page, link_ref, read, created_at
         FROM notifications ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, RETENTION_LIMIT))
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())
}

async fn count_unread(db: &Db) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notifications WHERE read = 0")
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

/// 收件箱视图：列表 + 未读数，一次取回避免前端两次往返。
#[derive(Debug, Clone, Serialize)]
pub struct NotificationInbox {
    pub items: Vec<Notification>,
    pub unread: i64,
}

#[tauri::command]
pub async fn list_notifications(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<NotificationInbox, String> {
    reconcile_stale(&state.db).await?;
    let items = fetch_recent(&state.db, limit.unwrap_or(50)).await?;
    let unread = count_unread(&state.db).await?;
    Ok(NotificationInbox { items, unread })
}

#[tauri::command]
pub async fn unread_notification_count(state: State<'_, AppState>) -> Result<i64, String> {
    reconcile_stale(&state.db).await?;
    count_unread(&state.db).await
}

#[tauri::command]
pub async fn mark_notification_read(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    sqlx::query("UPDATE notifications SET read = 1 WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn mark_all_notifications_read(state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("UPDATE notifications SET read = 1 WHERE read = 0")
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
