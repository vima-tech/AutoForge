//! 工厂自喂料：proposer Agent 基于项目现状主动提议改进，产物进 triage 池。
//!
//! 纯 Rust、零 Tauri 类型：本模块只**生成** `IntakePayload`，由调用方（命令/调度器）
//! 经 `gateway::receive(.., IntakeMode::Triage)` 落库——既复用去重+注入过滤，也保证
//! 自喂料**永远只进待整理池**、绝不自动进流水线（安全护栏见 C4）。
//!
//! schema 驱动：一次 propose 运行的结构化产出整体落 `agent_outputs`
//! （role=proposer, target_kind=project, target_id=project_id）——proposer 在「propose 时」
//! 面向的实体是项目本身（issue 尚未创建），故按「一次运行一行」沉淀，trace_id 链回单步推理。

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::agents::schema::{self, StructuredSchema};
use crate::db::Db;
use crate::intake::IntakePayload;

fn default_category() -> String {
    "Improvement".to_string()
}
fn default_severity() -> String {
    "medium".to_string()
}

/// 一条工程/功能提议（proposer schema v1.0，批量数组元素）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProposalItem {
    #[serde(default)]
    pub title: String,
    /// engineering（带 file:line 证据）| feature（高优先级新功能，少量）。
    #[serde(default)]
    pub kind: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub impact: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub description: String,
}

/// 工程类提议的 file:line 证据项。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub note: String,
}

const PROPOSAL_SCHEMA_TEMPLATE: &str = r#"[{
  "title": "<简洁标题>", "kind": "<engineering|feature>",
  "category": "<Feature|Bug|Improvement|Debt>", "severity": "<critical|high|medium|low>",
  "rationale": "<为什么值得做>",
  "evidence": [{"file": "<相对路径>", "line": <行号或 null>, "note": "<该处的问题>"}],
  "impact": "<影响面>", "effort": "<S|M|L>", "description": "<详细说明>"
}]"#;

/// 整批提议的存储信封（落 `agent_outputs.output_json`）。
#[derive(Serialize)]
struct ProposalBatch<'a> {
    schema_version: &'a str,
    proposals: &'a [ProposalItem],
}

impl StructuredSchema for ProposalItem {
    const ROLE: &'static str = "proposer";
    const VERSION: &'static str = "1.0";
    fn schema_template() -> &'static str {
        PROPOSAL_SCHEMA_TEMPLATE
    }
}

impl Evidence {
    /// 渲染为 `file:line（note）` 形式，供拼进 issue 描述。
    fn render(&self) -> String {
        match self.line {
            Some(l) if !self.file.is_empty() => format!("{}:{}（{}）", self.file, l, self.note),
            _ if !self.file.is_empty() => format!("{}（{}）", self.file, self.note),
            _ => self.note.clone(),
        }
    }
}

/// 运行 proposer，返回至多 `max` 条提议载荷（source_type=proposer）。
/// 失败（未配置角色/LLM、解析失败）返回 Err 或空向量，由调用方静默处理。
/// 同时把整批结构化产出落 `agent_outputs`（best-effort）。
pub async fn propose(db: &Db, project_id: &str, max: usize) -> Result<Vec<IntakePayload>> {
    let max = max.clamp(1, 20);
    let instruction = format!(
        "请基于本项目的代码现状与上下文，提出最多 {} 条最值得做的改进/需求。\
         工程类每条必须带 file:line 证据，宁缺毋滥。",
        max
    );
    let (raw, trace_id) = crate::agents::llm::run_system_role_text_traced(
        db,
        "proposer",
        &instruction,
        None,
        Some(project_id),
        None,
    )
    .await?;

    let (items, status) = schema::parse_array_or_empty::<ProposalItem>(&raw);

    // 落库整批结构化产出：一次运行一行，target=该项目。
    let envelope = ProposalBatch {
        schema_version: ProposalItem::VERSION,
        proposals: &items,
    };
    let output_json = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".to_string());
    schema::record(
        db,
        ProposalItem::ROLE,
        ProposalItem::VERSION,
        "project",
        project_id,
        Some(project_id),
        trace_id.as_deref(),
        status,
        &output_json,
        &raw,
    )
    .await;

    Ok(items
        .into_iter()
        .filter(|p| !p.title.trim().is_empty())
        .take(max)
        .map(|p| {
            let evidence = p
                .evidence
                .iter()
                .map(Evidence::render)
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("；");
            let mut desc = if p.description.trim().is_empty() {
                p.rationale.clone()
            } else {
                p.description.clone()
            };
            if !evidence.is_empty() {
                desc = format!("{}\n\n— 证据：{}", desc, evidence);
            }
            IntakePayload {
                project_id: project_id.to_string(),
                title: p.title,
                description: Some(desc),
                category: Some(p.category),
                severity: Some(p.severity),
                source_type: "proposer".to_string(),
                source_ref: None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // §3.5 防漂移 + 证据结构化：规范样例含 file:line 证据对象，必须解析进结构。
    #[test]
    fn proposal_parses_structured_evidence() {
        let ex = r#"前言[
          {"title":"补充网络重试","kind":"engineering","category":"Improvement","severity":"medium",
           "rationale":"弱网下请求易失败","evidence":[{"file":"src/net.rs","line":42,"note":"无重试"}],
           "impact":"稳定性","effort":"S","description":"为关键请求加指数退避重试"}
        ]尾部"#;
        let (items, st) = schema::parse_array_or_empty::<ProposalItem>(ex);
        assert_eq!(st, "ok");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].evidence.len(), 1);
        assert_eq!(items[0].evidence[0].line, Some(42));
        assert_eq!(items[0].evidence[0].render(), "src/net.rs:42（无重试）");
        assert_eq!(<ProposalItem as StructuredSchema>::ROLE, "proposer");
    }

    #[test]
    fn evidence_render_variants() {
        assert_eq!(
            Evidence { file: String::new(), line: None, note: "纯说明".into() }.render(),
            "纯说明"
        );
        assert_eq!(
            Evidence { file: "a.rs".into(), line: None, note: "无行号".into() }.render(),
            "a.rs（无行号）"
        );
    }
}
