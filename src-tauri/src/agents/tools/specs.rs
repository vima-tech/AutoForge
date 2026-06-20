//! 项目规格工具组——让 Agent 以 skill 式「渐进读取」消费项目规格，而非全量注入。
//!
//! 两个**只读、无副作用**工具（符合 CLAUDE.md「MVP 只读工具」铁律）：
//! - `list_specs` ：列出本项目可读规格的目录（标题 + 分类 + 描述 + id），便宜的「声明」。
//! - `read_spec`  ：按 id 或标题拉取某条规格全文（db 源读内联 content，file 源经守卫读盘）。
//!
//! 注入档位语义：`always`/`on_demand` 的规格出现在 `list_specs`；`off` 完全不暴露。
//! 持有 `db + project_id + repo_root`（来自 [`ToolContext`]），不引用任何 Tauri 类型。
//! 返回内容是不可信外部输入，由 [`super::ToolRegistry::invoke`] 统一过注入闸 + 截断。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::{BuiltinTool, Tool, ToolContext, ToolInfo, ToolSpec};
use crate::db::Db;
use crate::models::spec::ProjectSpec;

/// 规格工具工厂：两个工具共用 db + project_id + repo_root。
/// 无项目（project_id 缺失）时 `build` 返回 None。
pub enum SpecToolFactory {
    List,
    Read,
}

#[async_trait]
impl BuiltinTool for SpecToolFactory {
    fn info(&self) -> ToolInfo {
        match self {
            SpecToolFactory::List => ToolInfo {
                name: "list_specs",
                label: "列项目规格",
                needs_project: true,
            },
            SpecToolFactory::Read => ToolInfo {
                name: "read_spec",
                label: "读项目规格",
                needs_project: true,
            },
        }
    }

    async fn build(&self, db: &Db, ctx: &ToolContext) -> Option<Arc<dyn Tool>> {
        let project_id = ctx.project_id.clone()?;
        let repo_root = ctx.repo_root.clone();
        Some(match self {
            SpecToolFactory::List => {
                Arc::new(ListSpecsTool { db: db.clone(), project_id }) as Arc<dyn Tool>
            }
            SpecToolFactory::Read => {
                Arc::new(ReadSpecTool { db: db.clone(), project_id, repo_root }) as Arc<dyn Tool>
            }
        })
    }
}

/// 分类 key → 中文显示名（与 commands/specs.rs 的 CATEGORIES 一致）。
fn category_display(category: &str) -> &str {
    match category {
        "tech_stack" => "技术栈",
        "architecture" => "架构约束",
        "coding" => "编码规范",
        "api" => "API 契约",
        "testing" => "测试要求",
        "reference" => "参考",
        other => other,
    }
}

/// 规整为 `.autoforge/` 相对路径（如 `specs/foo.md`），供 workspace 守卫复用。
fn to_workspace_rel(rel_path: &str) -> String {
    let p = rel_path.trim().trim_start_matches("./");
    p.strip_prefix(".autoforge/").unwrap_or(p).to_string()
}

// ── list_specs ────────────────────────────────────────────────────────────────

struct ListSpecsTool {
    db: Db,
    project_id: String,
}

#[async_trait]
impl Tool for ListSpecsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "list_specs",
            "列出当前项目可按需读取的规格目录（标题、分类、描述、id）。先用本工具了解有哪些规格，再用 read_spec 拉取需要的那条全文，避免一次把所有规格塞进上下文。无参数。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        )
    }

    async fn call(&self, _args: Value) -> Result<String> {
        let specs = sqlx::query_as::<_, ProjectSpec>(
            "SELECT * FROM project_specs
             WHERE project_id = ? AND injection IN ('always','on_demand')
             ORDER BY category, sort_order, created_at",
        )
        .bind(&self.project_id)
        .fetch_all(&self.db)
        .await
        .map_err(|e| anyhow!("查询规格失败: {}", e))?;

        if specs.is_empty() {
            return Ok("当前项目暂无可读规格。".to_string());
        }

        let mut out = String::from("当前项目可读规格（用 read_spec 按 id 或标题拉全文）：\n");
        for s in &specs {
            let src = if s.source == "file" { "文件" } else { "DB" };
            let desc = if s.description.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", s.description.trim())
            };
            out.push_str(&format!(
                "- [{}/{}] {}{} (id={})\n",
                category_display(&s.category),
                src,
                s.title,
                desc,
                s.id,
            ));
        }
        Ok(out)
    }
}

// ── read_spec ─────────────────────────────────────────────────────────────────

struct ReadSpecTool {
    db: Db,
    project_id: String,
    repo_root: Option<PathBuf>,
}

#[async_trait]
impl Tool for ReadSpecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "read_spec",
            "读取当前项目某一条规格的全文。优先用 id 精确匹配（来自 list_specs），也可用 title 标题匹配。",
            json!({
                "type": "object",
                "properties": {
                    "id":    { "type": "string", "description": "规格 id（来自 list_specs，最精确）" },
                    "title": { "type": "string", "description": "规格标题（当没有 id 时按标题匹配，不区分大小写）" }
                },
                "additionalProperties": false
            }),
        )
    }

    async fn call(&self, args: Value) -> Result<String> {
        let id = args.get("id").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
        let title = args.get("title").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());

        // 解析目标规格（排除 off：不暴露）。
        let spec: Option<ProjectSpec> = if let Some(id) = id {
            sqlx::query_as::<_, ProjectSpec>(
                "SELECT * FROM project_specs
                 WHERE id = ? AND project_id = ? AND injection != 'off'",
            )
            .bind(id)
            .bind(&self.project_id)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| anyhow!("查询规格失败: {}", e))?
        } else if let Some(title) = title {
            sqlx::query_as::<_, ProjectSpec>(
                "SELECT * FROM project_specs
                 WHERE project_id = ? AND injection != 'off' AND lower(title) = lower(?)
                 ORDER BY sort_order LIMIT 1",
            )
            .bind(&self.project_id)
            .bind(title)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| anyhow!("查询规格失败: {}", e))?
        } else {
            return Err(anyhow!("请提供 id 或 title 之一"));
        };

        let spec = spec.ok_or_else(|| anyhow!("未找到匹配的规格（可能不存在或被设为不暴露）"))?;

        let body = if spec.source == "file" {
            let repo_root = self
                .repo_root
                .as_ref()
                .ok_or_else(|| anyhow!("项目仓库路径不可用，无法读取文件规格"))?;
            let rel = to_workspace_rel(&spec.rel_path);
            crate::commands::workspace::read_workspace_path(&repo_root.to_string_lossy(), &rel)
                .await
                .map_err(|e| anyhow!("读取文件规格失败: {}", e))?
        } else {
            spec.content.clone()
        };

        Ok(format!(
            "# {} [{}]\n{}\n\n{}",
            spec.title,
            category_display(&spec.category),
            if spec.description.trim().is_empty() {
                String::new()
            } else {
                format!("> {}", spec.description.trim())
            },
            body
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool_with_specs() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE project_specs (
                id TEXT PRIMARY KEY, project_id TEXT, category TEXT, title TEXT,
                content TEXT DEFAULT '', sort_order INTEGER DEFAULT 0,
                source TEXT DEFAULT 'db', rel_path TEXT DEFAULT '',
                description TEXT DEFAULT '', injection TEXT DEFAULT 'always',
                created_at TEXT, updated_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        // 三条：always(db) / on_demand(db) / off(db)
        for (id, title, content, injection) in [
            ("s1", "双审核节点", "必须两个人工审核节点", "always"),
            ("s2", "工具循环上限", "工具调用最多 6 轮", "on_demand"),
            ("s3", "隐藏项", "不应暴露", "off"),
        ] {
            sqlx::query(
                "INSERT INTO project_specs
                 (id, project_id, category, title, content, source, injection, created_at, updated_at)
                 VALUES (?, 'p1', 'architecture', ?, ?, 'db', ?, '', '')",
            )
            .bind(id)
            .bind(title)
            .bind(content)
            .bind(injection)
            .execute(&p)
            .await
            .unwrap();
        }
        p
    }

    #[tokio::test]
    async fn list_specs_excludes_off() {
        let db = pool_with_specs().await;
        let tool = ListSpecsTool { db, project_id: "p1".into() };
        let out = tool.call(json!({})).await.unwrap();
        assert!(out.contains("双审核节点"), "应含 always 项");
        assert!(out.contains("工具循环上限"), "应含 on_demand 项");
        assert!(!out.contains("隐藏项"), "off 项不应出现在清单");
    }

    #[tokio::test]
    async fn read_spec_by_id_and_title() {
        let db = pool_with_specs().await;
        let tool = ReadSpecTool { db, project_id: "p1".into(), repo_root: None };
        let by_id = tool.call(json!({ "id": "s2" })).await.unwrap();
        assert!(by_id.contains("工具调用最多 6 轮"));
        let by_title = tool.call(json!({ "title": "双审核节点" })).await.unwrap();
        assert!(by_title.contains("必须两个人工审核节点"));
    }

    #[tokio::test]
    async fn read_spec_off_is_hidden() {
        let db = pool_with_specs().await;
        let tool = ReadSpecTool { db, project_id: "p1".into(), repo_root: None };
        let res = tool.call(json!({ "id": "s3" })).await;
        assert!(res.is_err(), "off 规格不可读取");
    }
}
