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

/// 分析视角（lens）：把单一「提改进」拆成多个聚焦的深度审查角度，各自带定向 directive +
/// 量身定制的种子上下文。一轮 propose 会依次跑全部 lens，让产出覆盖**功能异常 / 逻辑漏洞 /
/// 需求建议**三类，而非泛泛一问——这是「深度」的核心来源。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lens {
    /// 规格符合性：对照规格 + 已交付 CR 找未实现、半成品、回退的功能（功能异常）。
    SpecConformance,
    /// 逻辑不变量：沿变更热点模块深挖并发/错误处理/资源/契约/状态机等逻辑漏洞。
    Logic,
    /// 需求建议：从产品视角提少量高价值的新功能/体验改进。
    Requirement,
}

impl Lens {
    const ALL: [Lens; 3] = [Lens::SpecConformance, Lens::Logic, Lens::Requirement];

    /// lens 标识：落 `agent_outputs.target_id` 后缀 + `IntakePayload.source_ref`，便于区分来源。
    fn key(self) -> &'static str {
        match self {
            Lens::SpecConformance => "spec",
            Lens::Logic => "logic",
            Lens::Requirement => "requirement",
        }
    }

    /// 本视角的定向指令（叠加在角色 system 提示之上，聚焦本轮要找的缺陷类）。
    fn directive(self) -> &'static str {
        match self {
            Lens::SpecConformance =>
                "本轮聚焦【功能异常 / 半成品】：对照项目规格与近期已合并的需求（声称已交付），\
                 核实它们是否真正、完整地落地。用 list_specs/read_spec 读规格条目，用 \
                 search_project_code/read_project_file 定位实现，重点找：① 规格要求但代码缺失或仅 \
                 stub（todo!/unimplemented!/空实现/未接线 UI）；② 已合并需求声称完成、实际回退或半途而废；\
                 ③ 状态机/分支「声明了却走不到」的死路。每条 engineering 带真实 file:line 证据。",
            Lens::Logic =>
                "本轮聚焦【逻辑漏洞】：在下列变更热点模块里深挖并发竞态、被吞掉的错误、资源泄漏、\
                 幂等缺失、数据/接口契约违背、非法状态跃迁。先 read_project_file 通读可疑函数及其调用关系，\
                 再断言『什么条件下会出什么错』。优先彻查『本轮深挖焦点』那个模块，每条带 file:line 证据与触发条件。",
            Lens::Requirement =>
                "本轮聚焦【需求建议】：从产品与可用性视角，提出**至多 2 条**真正高价值的新功能或体验改进。\
                 可不带 file:line 证据，但必须说清用户价值与场景。严禁提皮毛/凑数，没有高价值想法就返回 []。",
        }
    }
}

/// 把预算 `total` 近似均分到 `n` 个 lens（前置 lens 多分余数）。
fn split_budget(total: usize, n: usize) -> Vec<usize> {
    if n == 0 {
        return vec![];
    }
    let base = total / n;
    let rem = total % n;
    (0..n).map(|i| base + usize::from(i < rem)).collect()
}

/// 运行 proposer：依次跑全部 lens（规格符合性 / 逻辑 / 需求），合并产出 → 自验证（杀掉证据
/// 对不上的幻觉条目）→ 批内去重 → 截断到 `max`。返回至多 `max` 条提议载荷（source_type=proposer）。
/// 每个 lens 的结构化产出各落一行 `agent_outputs`（best-effort），target_id 带 lens 后缀。
pub async fn propose(db: &Db, project_id: &str, max: usize) -> Result<Vec<IntakePayload>> {
    let max = max.clamp(1, 20);
    let repo_path = repo_path_of(db, project_id).await;
    let budgets = split_budget(max, Lens::ALL.len());
    let mut all: Vec<(Lens, ProposalItem)> = Vec::new();

    for (lens, budget) in Lens::ALL.iter().zip(budgets) {
        if budget == 0 {
            continue;
        }
        let seed = build_lens_seed(db, project_id, repo_path.as_deref(), *lens).await;
        let instruction = format!(
            "{directive}\n\n请最多提出 {budget} 条。务必先用 search_project_code/read_project_file/read_spec \
             核实再下结论；宁缺毋滥，没有带证据的真问题就返回空数组 []。\n\n{seed}",
            directive = lens.directive(),
            budget = budget,
            seed = seed,
        );
        let (raw, trace_id) = match crate::agents::llm::run_system_role_text_traced(
            db,
            "proposer",
            &instruction,
            None,
            Some(project_id),
            None,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[proposer] lens={} 调用失败：{}", lens.key(), e);
                continue;
            }
        };

        let (items, status) = schema::parse_array_or_empty::<ProposalItem>(&raw);
        // 落库：每个 lens 一行，target_id=「项目#lens」便于区分。
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
            &format!("{}#{}", project_id, lens.key()),
            Some(project_id),
            trace_id.as_deref(),
            status,
            &output_json,
            &raw,
        )
        .await;

        for it in items.into_iter().filter(|p| !p.title.trim().is_empty()) {
            all.push((*lens, it));
        }
    }

    // 自验证：丢弃证据对不上的 engineering 条目（杀掉幻觉行号/不存在的文件）。
    let (verified, dropped) = verify_evidence(repo_path.as_deref(), all).await;
    if dropped > 0 {
        eprintln!("[proposer] 自验证丢弃 {} 条证据不实的提议", dropped);
    }

    // 批内去重（同标题）+ 截断到总预算。
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (lens, p) in verified {
        let key = p.title.trim().to_string();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        out.push(to_payload(project_id, lens, p));
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

/// 把一条提议映射为入池载荷：描述拼接证据，source_ref 标记来源 lens。
fn to_payload(project_id: &str, lens: Lens, p: ProposalItem) -> IntakePayload {
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
        source_ref: Some(format!("proposer:{}", lens.key())),
    }
}

/// 自验证（杀幻觉）：engineering 提议必须带证据，且至少一条证据文件真实存在、行号大致在范围内；
/// 否则丢弃。前瞻性 feature/需求（非 engineering）无需 file:line，原样保留。
/// 返回 `(保留项, 丢弃数)`。无 repo 路径时无从核实，全部保留（best-effort 不误杀）。
async fn verify_evidence(
    repo: Option<&str>,
    items: Vec<(Lens, ProposalItem)>,
) -> (Vec<(Lens, ProposalItem)>, usize) {
    let mut kept = Vec::new();
    let mut dropped = 0usize;
    for (lens, item) in items {
        let is_eng = item.kind.eq_ignore_ascii_case("engineering");
        let Some(repo) = repo.filter(|_| is_eng) else {
            kept.push((lens, item));
            continue;
        };
        if item.evidence.is_empty() {
            dropped += 1; // engineering 无证据 → 不可信，丢弃。
            continue;
        }
        let mut ok = false;
        for ev in &item.evidence {
            if evidence_exists(repo, ev).await {
                ok = true;
                break;
            }
        }
        if ok {
            kept.push((lens, item));
        } else {
            dropped += 1;
        }
    }
    (kept, dropped)
}

/// 校验单条证据：文件在 repo 内真实存在（拒绝 `..` 越界），且若给了行号则不超过文件行数（容忍少量偏移）。
async fn evidence_exists(repo: &str, ev: &Evidence) -> bool {
    let f = ev.file.trim();
    if f.is_empty() || f.contains("..") {
        return false;
    }
    let path = std::path::Path::new(repo).join(f);
    let Ok(content) = tokio::fs::read_to_string(&path).await else {
        return false;
    };
    match ev.line {
        Some(l) if l > 0 => l <= content.lines().count() as i64 + 3,
        _ => true,
    }
}

/// 按 lens 组装聚焦的种子上下文。best-effort——任何子项取不到就跳过，绝不阻断主流程。
/// 返回可直接拼进 instruction 的文本（空串表示无可用上下文）。
async fn build_lens_seed(
    db: &Db,
    project_id: &str,
    repo: Option<&str>,
    lens: Lens,
) -> String {
    let mut blocks: Vec<String> = Vec::new();

    match lens {
        Lens::SpecConformance => {
            let specs = spec_titles(db, project_id).await;
            if !specs.is_empty() {
                blocks.push(format!(
                    "### 项目规格条目（用 read_spec 读全文，逐条核对代码是否真正落地）\n{}",
                    bullet(&specs)
                ));
            }
            let merged = merged_titles(db, project_id).await;
            if !merged.is_empty() {
                blocks.push(format!(
                    "### 近期已合并需求（声称已交付——核实是否完整落地，警惕半成品/回退/被绕过）\n{}",
                    bullet(&merged)
                ));
            }
        }
        Lens::Logic => {
            if let Some(r) = repo {
                let recent = git_lines(r, &["log", "--oneline", "-n", "12"]).await;
                if !recent.is_empty() {
                    blocks.push(format!("### 近期提交\n{}", recent.join("\n")));
                }
                let hotspots = git_hotspots(r, 12).await;
                if !hotspots.is_empty() {
                    // 轮转焦点：每轮深挖一个热点模块，多轮覆盖全仓（「扫地机器人」式窄而深）。
                    let cur = next_scope_cursor(db).await as usize;
                    let focus = &hotspots[cur % hotspots.len()];
                    blocks.push(format!("### 本轮深挖焦点（优先彻查此模块）\n{}", focus));
                    blocks.push(format!(
                        "### 其余变更热点（次优先，改动最频繁=风险最集中）\n{}",
                        hotspots.join("\n")
                    ));
                }
            }
        }
        Lens::Requirement => {
            if let Some(r) = repo {
                let docs = list_docs(r);
                if !docs.is_empty() {
                    blocks.push(format!(
                        "### 产品文档（.autoforge/docs，可 read_project_file 参考产品意图）\n{}",
                        bullet(&docs)
                    ));
                }
            }
            let merged = merged_titles(db, project_id).await;
            if !merged.is_empty() {
                blocks.push(format!("### 已交付能力（避免重复建议）\n{}", bullet(&merged)));
            }
        }
    }

    // 所有 lens 共享：被人工否决的方向（负面样例，反馈闭环——别再重复提）。
    let rejected = rejected_titles(db, project_id).await;
    if !rejected.is_empty() {
        blocks.push(format!(
            "### 已被人工否决的方向（视为低价值，**不要再提**类似的）\n{}",
            bullet(&rejected)
        ));
    }

    if blocks.is_empty() {
        String::new()
    } else {
        format!("## 种子上下文（聚焦本轮视角）\n\n{}", blocks.join("\n\n"))
    }
}

/// 把字符串列表渲染为 markdown 无序列表。
fn bullet(items: &[String]) -> String {
    items
        .iter()
        .map(|t| format!("- {}", t.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 取项目 repo 路径（非空且存在才返回）。
async fn repo_path_of(db: &Db, project_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT repo_path FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .filter(|p| !p.trim().is_empty())
}

/// 项目规格条目标题（供规格符合性 lens 对照）。
async fn spec_titles(db: &Db, project_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT title FROM project_specs WHERE project_id=? ORDER BY updated_at DESC LIMIT 30",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// 近期已合并需求标题（= 声称已交付的能力）。
async fn merged_titles(db: &Db, project_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT title FROM issues WHERE project_id=? AND status='merged' ORDER BY updated_at DESC LIMIT 20",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// 被人工否决的需求标题（负面样例）。
async fn rejected_titles(db: &Db, project_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT title FROM issues WHERE project_id=? AND status='rejected' ORDER BY updated_at DESC LIMIT 15",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// 列出 `.autoforge/docs` 下的文档文件名（best-effort，单层目录，限量）。
fn list_docs(repo: &str) -> Vec<String> {
    let dir = std::path::Path::new(repo).join(".autoforge").join("docs");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .take(30)
        .collect()
}

/// 读取并自增「深挖焦点」轮转游标（存 app_settings），返回自增前的值。
/// 让逻辑 lens 每轮聚焦不同热点模块，多轮覆盖全仓。
async fn next_scope_cursor(db: &Db) -> i64 {
    const KEY: &str = "autosupply.proposer_scope_cursor";
    let cur = sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(KEY)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .rem_euclid(1_000_000); // 防溢出，循环范围足够大
    let next = cur + 1;
    let _ = sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(KEY)
    .bind(next.to_string())
    .execute(db)
    .await;
    cur
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
    fn split_budget_distributes_with_front_loaded_remainder() {
        assert_eq!(split_budget(8, 3), vec![3, 3, 2]);
        assert_eq!(split_budget(2, 3), vec![1, 1, 0]); // 需求 lens 让位给前两个
        assert_eq!(split_budget(6, 3), vec![2, 2, 2]);
        assert_eq!(split_budget(1, 3), vec![1, 0, 0]);
        assert_eq!(split_budget(0, 3), vec![0, 0, 0]);
    }

    #[test]
    fn lens_keys_are_stable_and_distinct() {
        let keys: Vec<&str> = Lens::ALL.iter().map(|l| l.key()).collect();
        assert_eq!(keys, vec!["spec", "logic", "requirement"]);
    }

    #[tokio::test]
    async fn evidence_exists_rejects_traversal_and_missing() {
        // `..` 越界一律判不存在（兼顾安全）。
        let ev = Evidence { file: "../etc/passwd".into(), line: None, note: String::new() };
        assert!(!evidence_exists("/tmp", &ev).await);
        // 不存在的文件 → false。
        let ev = Evidence { file: "definitely-missing-xyz.rs".into(), line: None, note: String::new() };
        assert!(!evidence_exists("/tmp", &ev).await);
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
