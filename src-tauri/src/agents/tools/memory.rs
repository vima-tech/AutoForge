//! Innate 记忆工具组——把 Innate 从「系统提示词背景注入」升级为 Agent 推理中途**主动调用**
//! 的一等能力，闭合「召回广但被动、捕获浅且只能流水线硬编码」的缺口。
//!
//! 两个工具，复用 [`crate::knowledge`] 既有 scope 感知入口（project_id 有 → 项目库，无 → 共享库）：
//! - `recall_memory`    ：按需召回与当前子问题相关的经验/技能（只读）。
//! - `remember_insight` ：把推理中刚得到的、可复用的非显然洞见沉淀为知识 chunk（写本地 Innate 库）。
//!
//! 安全/铁律对齐：
//! - 纯 Rust、零 Tauri 类型，只依赖 db / knowledge 纯模块。
//! - `recall_memory` 的返回是「我们自己的库」内容，仍由 [`super::ToolRegistry::invoke`] 统一过
//!   注入闸 + 截断，与其它工具同路径。
//! - `remember_insight` 是**写类工具**：默认不开启，必须在 Agent `capabilities_json` 白名单显式勾选
//!   才装配（符合 CLAUDE.md「写类工具走白名单」）。写入目标是受信的本地知识库、无外部副作用；
//!   `cmd_remember` 内部做长度上限 + 触发自动进化计数。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;

/// 记忆工具工厂：召回（只读）+ 沉淀（写）。两者都不强制绑定项目——
/// 无项目场景（直聊）落到共享「跨项目技艺」库，由 `knowledge` 的 scope 解析处理。
pub enum MemoryToolFactory {
    Recall,
    Remember,
}

#[async_trait]
impl BuiltinTool for MemoryToolFactory {
    fn info(&self) -> ToolInfo {
        match self {
            MemoryToolFactory::Recall => ToolInfo {
                name: "recall_memory",
                label: "召回记忆",
                needs_project: false,
            },
            MemoryToolFactory::Remember => ToolInfo {
                name: "remember_insight",
                label: "沉淀经验",
                needs_project: false,
            },
        }
    }

    async fn build(&self, _db: &Db, ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        let project_id = ctx.project_id.clone();
        Some(match self {
            MemoryToolFactory::Recall => {
                Arc::new(RecallMemoryTool { project_id }) as Arc<dyn Tool>
            }
            MemoryToolFactory::Remember => {
                Arc::new(RememberInsightTool { project_id }) as Arc<dyn Tool>
            }
        })
    }
}

// ── recall_memory ───────────────────────────────────────────────────────────────

struct RecallMemoryTool {
    project_id: Option<String>,
}

#[async_trait]
impl Tool for RecallMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "recall_memory",
            "按需召回与当前子问题最相关的历史经验/技能/踩坑记录（来自本项目库 + 跨项目共享库）。\
             当你遇到一个具体决策点（用哪个 API、某类报错怎么修、某个约定怎么落地）且系统提示里现有上下文不够时调用；\
             query 用尽量具体的关键词（如「sqlx 事务回滚」而非「数据库」）。只读、无副作用。",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "要召回的具体问题/关键词，越聚焦命中越准" }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("请提供非空的 query"))?;

        let text = crate::knowledge::cmd_recall(self.project_id.as_deref(), query)
            .await
            .map_err(|e| anyhow!(e))?;
        if text.trim().is_empty() {
            Ok("未召回到相关经验（库中暂无匹配条目）。".to_string())
        } else {
            Ok(text)
        }
    }
}

// ── remember_insight ─────────────────────────────────────────────────────────────

struct RememberInsightTool {
    project_id: Option<String>,
}

#[async_trait]
impl Tool for RememberInsightTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "remember_insight",
            "把本次推理中刚得到的、**可复用且非显然**的经验沉淀为长期知识（写入本项目/共享 Innate 库），\
             供未来同类任务自动召回。仅在确有值得记的结论时调用：踩过的坑、被验证的做法、项目特有约定/不变量。\
             不要记录一次性、显而易见或会很快过期的内容。",
            json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "要记住的经验本体，一句话讲清「结论」，必要时附「为什么」" },
                    "trigger": { "type": "string", "description": "「何时该应用这条经验」的场景描述（驱动召回命中），如「写 SQLite 迁移时」。省略则由内容自动推断" }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("请提供非空的 content"))?;
        // trigger 缺省时用内容前缀兜底，保证仍有「何时应用」的可召回向量。
        let trigger = args
            .get("trigger")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| content.chars().take(80).collect());

        crate::knowledge::cmd_remember(self.project_id.as_deref(), content, &trigger)
            .await
            .map_err(|e| anyhow!(e))?;
        Ok(format!("已沉淀经验（触发场景：{}）。", trigger))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 入参校验在触达知识库前完成 → 不依赖 kb_base 初始化即可测。
    #[tokio::test]
    async fn recall_rejects_empty_query() {
        let tool = RecallMemoryTool { project_id: Some("p1".into()) };
        assert!(tool.call(json!({ "query": "  " })).await.is_err());
        assert!(tool.call(json!({})).await.is_err());
    }

    #[tokio::test]
    async fn remember_rejects_empty_content() {
        let tool = RememberInsightTool { project_id: None };
        assert!(tool.call(json!({ "content": "" })).await.is_err());
        assert!(tool.call(json!({ "trigger": "x" })).await.is_err());
    }

    #[test]
    fn specs_are_wellformed() {
        let r = RecallMemoryTool { project_id: None }.spec();
        assert_eq!(r.name, "recall_memory");
        assert!(r.parameters.get("properties").is_some());
        let w = RememberInsightTool { project_id: None }.spec();
        assert_eq!(w.name, "remember_insight");
        assert_eq!(w.parameters["required"][0], "content");
    }
}
