//! 上下文基质取用工具（消费侧）——让 Agent 按需从统一基质拉取「之前任意环节创建的
//! 物料 / 过程信息」（需求、编码执行日志、孵化台草稿、会议室发言、审核意见……）。
//!
//! 这是方法论平台原则二「任何环节都能取用之前创建的一切上下文」的**消费侧兑现**：
//! 基质 register 钩子在各写入路径沉淀条目（issue/clog/bp/atr/crv），本工具让 Agent 主动取用。
//!
//! 只读、无副作用（CLAUDE.md「MVP 只读工具」铁律）。持 db + project_id（来自 [`ToolContext`]），
//! 不引用任何 Tauri 类型。返回内容由 [`super::ToolRegistry::invoke`] 统一过注入闸 + 截断。

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::core::context;
use crate::db::Db;

/// `recall_context` 工具工厂：无项目时 `build` 返回 None（基质按项目隔离）。
pub struct RecallContextFactory;

#[async_trait]
impl BuiltinTool for RecallContextFactory {
    fn info(&self) -> ToolInfo {
        ToolInfo {
            name: "recall_context",
            label: "取用上下文基质",
            needs_project: true,
        }
    }

    async fn build(&self, db: &Db, ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        let project_id = ctx.project_id.clone()?;
        Some(Arc::new(RecallContextTool {
            db: db.clone(),
            project_id,
        }) as Arc<dyn Tool>)
    }
}

struct RecallContextTool {
    db: Db,
    project_id: String,
}

#[async_trait]
impl Tool for RecallContextTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "recall_context",
            "从本项目的统一「上下文基质」按需取用之前任意环节沉淀的物料 / 过程信息\
             （需求 issue、编码执行日志 code_agent_log、孵化台草稿 incubator_draft、\
             会议室 Agent 发言 agent_output、审核意见 cr_review 等）。返回按来源类型分组的\
             候选条目 + 正文摘要，供你判断哪些与当前任务相关。",
            json!({
                "type": "object",
                "properties": {
                    "kinds": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "按来源类型过滤（可选）：issue / code_agent_log / incubator_draft / agent_output / cr_review 等；留空 = 不限来源"
                    },
                    "limit": { "type": "integer", "description": "最多返回条数（默认 8，上限 30）" }
                }
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let kinds: Vec<String> = args
            .get("kinds")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let kr: Vec<&str> = kinds.iter().map(|s| s.as_str()).collect();
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(8).clamp(1, 30);

        let items = context::list(&self.db, &self.project_id, &kr, limit).await?;
        if items.is_empty() {
            return Ok("（基质中暂无匹配的上下文条目）".to_string());
        }
        let mut out = String::new();
        for it in &items {
            // 每条附一段短摘要（大体量来源自动走保尾摘要），便于 Agent 判断相关性；
            // 需要全文时可据 id 的 content_ref 语义进一步取用。
            let snippet = context::fetch_content(&self.db, it, 400)
                .await
                .unwrap_or_default();
            let snippet = snippet.trim();
            out.push_str(&format!(
                "- [{}] {}（{}）\n  {}\n",
                it.source_kind,
                it.title,
                it.id,
                if snippet.is_empty() { "（无正文）" } else { snippet }
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::{register, source_kind, NewContextItem};

    async fn pool() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
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
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&p)
        .await
        .unwrap();
        p
    }

    #[tokio::test]
    async fn recall_lists_registered_items() {
        let db = pool().await;
        register(
            &db,
            NewContextItem::trusted("p1", source_kind::ISSUE, "i1", "登录页需求", ""),
        )
        .await
        .unwrap();
        register(
            &db,
            NewContextItem::trusted("p1", source_kind::CR_REVIEW, "cr1", "审核意见 · 代码审核", ""),
        )
        .await
        .unwrap();

        let tool = RecallContextTool {
            db,
            project_id: "p1".into(),
        };
        let out = tool.call(json!({})).await.unwrap();
        assert!(out.contains("登录页需求"));
        assert!(out.contains("审核意见"));
        assert!(out.contains("issue:i1"));

        // 按来源过滤：只要 cr_review。
        let only = tool.call(json!({"kinds": ["cr_review"]})).await.unwrap();
        assert!(only.contains("审核意见"));
        assert!(!only.contains("登录页需求"));
    }

    #[tokio::test]
    async fn recall_empty_project_is_graceful() {
        let db = pool().await;
        let tool = RecallContextTool {
            db,
            project_id: "empty".into(),
        };
        let out = tool.call(json!({})).await.unwrap();
        assert!(out.contains("暂无匹配"));
    }
}
