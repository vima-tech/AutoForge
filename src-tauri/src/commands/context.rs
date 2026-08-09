//! 上下文基质的只读 IPC 出口（薄包装 `core::context`）。
//!
//! 供前端「取景框（Lens）」界面枚举/装配/懒取上下文条目——即基质设计 §4.1 的三种访问。
//! 命令保持薄包装：只做「取 state → 调纯 Rust core → 返回」，业务逻辑全在 `core::context`。

use crate::core::context::{self, ContextRequest};
use crate::models::context_item::ContextItem;
use crate::state::AppState;
use tauri::State;

/// 统一 dispatch 出口（DUAL_HEAD M2 机制的 Tauri 对接点）：前端经此调用注册表里走统一
/// 契约的命令。**对接层不枚举命令名**（DUAL_HEAD 红线）——此函数只做「建 Ctx → 查全局
/// 注册表 → 分发」，加/删注册表命令时本函数 diff 为 0。当前注册表含基质只读命令
/// `ctx.list` / `ctx.assemble` / `ctx.fetch`；M2 逐域迁移时更多命令自动经此可达。
#[tauri::command]
pub async fn rpc_dispatch(
    cmd: String,
    args: serde_json::Value,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let ctx = crate::core::rpc::Ctx {
        state: std::sync::Arc::new(state.inner().clone()),
        sink: std::sync::Arc::new(app),
        principal: crate::core::rpc::Principal::local_owner(),
    };
    crate::core::rpc::global_registry()
        .dispatch(&cmd, ctx, args)
        .await
        .map_err(|e| e.to_string())
}

/// 枚举一个项目的上下文条目（薄索引元数据；access #1）。
/// `kinds` 为空 = 不限来源；`limit` 默认 200；`query` 有值 = 标题关键词搜索
/// （下推 provider 全量召回，结果按 kind 优先级 + 时间排序）。
#[tauri::command]
pub async fn list_context_items(
    project_id: String,
    kinds: Option<Vec<String>>,
    limit: Option<i64>,
    query: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ContextItem>, String> {
    let kinds_v = kinds.unwrap_or_default();
    let kinds_ref: Vec<&str> = kinds_v.iter().map(|s| s.as_str()).collect();
    context::list(
        &state.db,
        &project_id,
        &kinds_ref,
        limit.unwrap_or(200),
        query.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// 按取景框装配上下文条目（refs 置顶 → kind 优先级 → 预算裁剪；access #1 + §4.2）。
#[tauri::command]
pub async fn assemble_context(
    project_id: String,
    include: Option<Vec<String>>,
    refs: Option<Vec<String>>,
    budget_bytes: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<ContextItem>, String> {
    let req = ContextRequest {
        project_id,
        include: include.unwrap_or_default(),
        refs: refs.unwrap_or_default(),
        // None = 默认预算兜底；Some(0) = 显式不限（逃生口）。
        budget_bytes: budget_bytes.unwrap_or(context::DEFAULT_BUDGET_BYTES),
    };
    context::assemble(&state.db, &req)
        .await
        .map_err(|e| e.to_string())
}

/// 「引用到会议室」的落库逻辑（context_ref 块的产出方，G4 活体化，可测）：
/// 在目标会议室插入一条含 context_ref 块的消息（from_agent=NULL 表操作者引用）。
pub(crate) async fn insert_context_ref_message(
    db: &crate::db::Db,
    conversation_id: &str,
    item: &ContextItem,
) -> Result<String, String> {
    let content_json =
        serde_json::to_string(&vec![context::context_ref_block(item)]).map_err(|e| e.to_string())?;
    let msg_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO messages (id, conversation_id, from_agent, content_json) VALUES (?, ?, NULL, ?)")
        .bind(&msg_id)
        .bind(conversation_id)
        .bind(&content_json)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(msg_id)
}

/// 从取景框把一条基质条目引用到指定会议室（用户 pin：让基质条目进入协作对话，可展开取正文）。
#[tauri::command]
pub async fn cite_context_to_conversation(
    conversation_id: String,
    context_item_id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let item = context::get(&state.db, &context_item_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("上下文条目不存在")?;
    let msg_id = insert_context_ref_message(&state.db, &conversation_id, &item).await?;
    crate::core::event::emit(
        &app,
        crate::core::event::AppEvent::MessageReceived {
            conversation_id,
            message_id: msg_id,
        },
    );
    Ok(())
}

/// 懒取一条上下文条目正文（大体量来源走 G3 尾部摘要；access #2）。`max_chars` 默认 8192。
#[tauri::command]
pub async fn fetch_context_content(
    id: String,
    max_chars: Option<i64>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let item = context::get(&state.db, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("context item {} not found", id))?;
    context::fetch_content(&state.db, &item, max_chars.unwrap_or(8192) as usize)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::{register, source_kind, NewContextItem};

    /// context_ref 产出方：cite 把基质条目落成一条含 context_ref 块的会议室消息。
    #[tokio::test]
    async fn insert_context_ref_message_writes_block() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE context_index (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                origin_stage TEXT NOT NULL DEFAULT '', origin_actor TEXT NOT NULL DEFAULT '',
                content_ref TEXT NOT NULL DEFAULT '', size_hint INTEGER NOT NULL DEFAULT 0,
                trust TEXT NOT NULL DEFAULT 'trusted', labels TEXT NOT NULL DEFAULT '[]',
                created_at TEXT, updated_at TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE messages (id TEXT PRIMARY KEY, conversation_id TEXT, from_agent TEXT, content_json TEXT)")
            .execute(&db)
            .await
            .unwrap();
        register(&db, NewContextItem::trusted("p1", source_kind::ISSUE, "i1", "登录页需求", "issue:i1"))
            .await
            .unwrap();
        let item = context::get(&db, "issue:i1").await.unwrap().unwrap();

        let mid = insert_context_ref_message(&db, "conv1", &item).await.unwrap();
        assert!(!mid.is_empty());
        let (cj,): (String,) = sqlx::query_as("SELECT content_json FROM messages WHERE conversation_id='conv1'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(cj.contains("\"t\":\"context_ref\""));
        assert!(cj.contains("issue:i1"));
        assert!(cj.contains("登录页需求"));
    }
}
