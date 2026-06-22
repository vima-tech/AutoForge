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
    // 种子上下文：给审计师一张「地图」——近期变更热点（bug 高发区）、被人工否决过的
    // 方向（负面样例，别再提）。让它带着重点深挖，而非盲目 list 几个文件挑皮毛。
    let seed = build_seed_context(db, project_id).await;
    let instruction = format!(
        "请以资深代码审计师身份，深挖本项目 linter 发现不了的高价值问题，提出最多 {} 条。\
         务必先用 search_project_code/read_project_file 核实证据，每条 engineering 必须带真实 file:line 证据；\
         宁缺毋滥，没有深层问题就返回空数组 []。\n\n{}",
        max, seed
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

/// 组装 proposer 的种子上下文：近期变更热点（git）+ 被人工否决的负面样例（DB）。
/// best-effort——任何子项取不到就跳过，绝不阻断提议主流程。返回可直接拼进 instruction 的文本。
async fn build_seed_context(db: &Db, project_id: &str) -> String {
    let repo_path: Option<String> =
        sqlx::query_scalar::<_, String>("SELECT repo_path FROM projects WHERE id=?")
            .bind(project_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .filter(|p| !p.trim().is_empty());

    let mut blocks: Vec<String> = Vec::new();

    if let Some(repo) = repo_path.as_deref() {
        let recent = git_lines(repo, &["log", "--oneline", "-n", "12"]).await;
        if !recent.is_empty() {
            blocks.push(format!(
                "### 近期提交（优先审查最近改动的区域，bug 高发）\n{}",
                recent.join("\n")
            ));
        }
        let hotspots = git_hotspots(repo, 12).await;
        if !hotspots.is_empty() {
            blocks.push(format!(
                "### 变更热点文件（改动最频繁=风险最集中，重点审这些）\n{}",
                hotspots.join("\n")
            ));
        }
    }

    // 负面样例（P5 反馈闭环）：近期被人工否决/判噪的方向，别再重复提。
    let rejected: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT title FROM issues WHERE project_id=? AND status='rejected' \
         ORDER BY updated_at DESC LIMIT 15",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    if !rejected.is_empty() {
        blocks.push(format!(
            "### 已被人工否决的方向（视为低价值，**不要再提**类似的）\n{}",
            rejected
                .iter()
                .map(|t| format!("- {}", t))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if blocks.is_empty() {
        String::new()
    } else {
        format!("## 项目种子上下文（用于聚焦审查）\n\n{}", blocks.join("\n\n"))
    }
}

/// 运行只读 git 命令，返回非空 stdout 行（带 5s 超时）。失败/超时返回空。
async fn git_lines(repo: &str, args: &[&str]) -> Vec<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    cmd.stdin(std::process::Stdio::null());
    let fut = cmd.output();
    let out = match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
        Ok(Ok(o)) => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 用近 200 条提交的改动文件频次，算出 top-N 变更热点文件（纯 Rust 计数，不走 shell 管道）。
async fn git_hotspots(repo: &str, top: usize) -> Vec<String> {
    let files = git_lines(
        repo,
        &["log", "--name-only", "--pretty=format:", "-n", "200"],
    )
    .await;
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for f in files {
        // 跳过明显的非源码/生成物，聚焦真正的代码热点。
        if f.contains("node_modules/")
            || f.contains("/target/")
            || f.ends_with(".lock")
            || f.ends_with(".md")
        {
            continue;
        }
        *counts.entry(f).or_insert(0) += 1;
    }
    let mut ranked: Vec<(String, u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(top)
        .map(|(f, c)| format!("- {} （近 200 提交改动 {} 次）", f, c))
        .collect()
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
