use super::local_claude;
use crate::core::security;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

pub const SYSTEM_PROMPT: &str = r#"你是 AutoForge 的需求分析 Agent。你的输出会被直接结构化解析，并用于驱动 Claude Code 自动修改代码。
请严格只输出一个 JSON 对象（不要 markdown 代码块、不要任何解释文字），格式如下：

{
  "schema_version": "1.0",
  "triage": {
    "authenticity_score": <0.0-1.0 真实性>,
    "feasibility_score": <0.0-1.0 可行性>,
    "priority_suggestion": <1-10 优先级>,
    "category_suggestion": "<Feature|Bug|Debt|Improvement>",
    "severity_suggestion": "<low|medium|high|critical>",
    "is_duplicate": <true|false>,
    "duplicate_of": <疑似重复的需求标识，未知则 null>,
    "duplicate_hint": "<疑似重复的简述，否则空字符串>",
    "needs_changes": <true|false 该需求是否真的需要改动代码>,
    "no_change_reason": "<needs_changes 为 false 时，一句话说明为何无需改动；否则空字符串>",
    "analysis_summary": "<120字以内摘要>",
    "confidence": <0.0-1.0 本次分析整体置信度>
  },
  "understanding": {
    "problem_type": "<bug|feature|enhancement|refactor|tech_debt|question>",
    "restated_requirement": "<用实现视角重述需求，消除歧义>",
    "user_story": <字符串或 null>,
    "current_behavior": <bug 的当前错误行为，非 bug 为 null>,
    "expected_behavior": <期望行为，可 null>,
    "reproduction_steps": ["<bug 复现步骤>"],
    "background": <相关背景或 null>
  },
  "root_cause": <bug 类填对象，否则 null：{
    "hypothesis": "<最可能根因>",
    "evidence": ["<证据>"],
    "suspected_locations": [{"file": "<相对路径>", "symbol": <函数/组件名或 null>, "reason": "<为何怀疑>"}],
    "confidence": <0.0-1.0>
  }>,
  "scope": {
    "affected_modules": ["<模块>"],
    "affected_files": [{"path": "<相对仓库根路径>", "change_type": "<add|modify|delete|investigate>", "reason": "<原因>", "confidence": <0.0-1.0>}],
    "related_files": ["<需阅读但不一定改动的文件>"],
    "entry_points": ["<建议入手点：函数/路由/命令/UI 位置>"],
    "out_of_scope": ["<明确不在本次范围内>"],
    "blast_radius": "<isolated|module|cross_module|systemic>"
  },
  "implementation_plan": {
    "approach": "<总体实现思路 1-3 句>",
    "steps": [{"order": <整数>, "action": "<具体动作>", "target_files": ["<文件>"], "details": <要点或 null>}],
    "alternatives": [{"option": "<备选方案>", "tradeoff": "<取舍>"}],
    "data_model_changes": [{"kind": "<migration|model|none>", "description": "<描述>"}],
    "new_dependencies": ["<新增依赖，应尽量避免>"]
  },
  "acceptance_criteria": [{"id": "AC-1", "statement": "<可验证标准>", "verify": <验证方式或 null>}],
  "test_plan": {"unit": ["<用例>"], "integration": [], "manual": [], "edge_cases": []},
  "risks": [{"description": "<风险>", "severity": "<low|medium|high>", "mitigation": <缓解或 null>}],
  "constraints": {"must": ["<必须遵守，来自项目规范>"], "must_not": ["<禁止事项>"]},
  "assumptions": ["<分析所做假设>"],
  "open_questions": ["<需人工在审核1澄清的问题>"],
  "estimate": {"complexity": "<xs|s|m|l|xl>", "confidence": <0.0-1.0>, "rationale": <理由或 null>},
  "claude_code_brief": {
    "objective": "<一句话目标>",
    "instructions": ["<有序的命令式执行指令>"],
    "do": ["<务必遵守>"],
    "dont": ["<禁止>"],
    "files_to_touch": ["<预计改动文件路径>"],
    "definition_of_done": ["<完成判定清单>"]
  }
}

要求：
- 结合项目上下文（技术栈、目录结构、现有模块、CLAUDE.md/规范）判断 scope 与 constraints。
- affected_files / suspected_locations 的 path 必须是相对仓库根的真实路径，定位越准越好。
- claude_code_brief 是 scope + implementation_plan 的蒸馏，必须自洽、可直接执行。
- 信息不足时，在 open_questions / assumptions 中说明，并相应降低 confidence，不要臆造。
- bug 类必须给出 root_cause 与 reproduction_steps；非 bug 类 root_cause 用 null。
- needs_changes：当需求确实需要改动代码时为 true。若判定为「误报/无需改动」——例如所指为测试夹具或示例数据、功能已实现、纯属提问或咨询、描述的问题在当前代码中不存在——则置 false，并在 no_change_reason 用一句话说明，同时 scope.affected_files 应为空。此时 AutoForge 将默认不建议进入编码执行。
"#;

// ── 完整结构化分析规格（对齐 schemas/issue_analysis.schema.json v1.0）──────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Triage {
    #[serde(default = "half")]
    pub authenticity_score: f64,
    #[serde(default = "half")]
    pub feasibility_score: f64,
    #[serde(default = "five")]
    pub priority_suggestion: i64,
    #[serde(default = "cat_default")]
    pub category_suggestion: String,
    #[serde(default = "sev_default")]
    pub severity_suggestion: String,
    #[serde(default)]
    pub is_duplicate: bool,
    #[serde(default)]
    pub duplicate_of: Option<String>,
    #[serde(default)]
    pub duplicate_hint: String,
    #[serde(default = "yes")]
    pub needs_changes: bool,
    #[serde(default)]
    pub no_change_reason: String,
    #[serde(default)]
    pub analysis_summary: String,
    #[serde(default = "half")]
    pub confidence: f64,
}
fn half() -> f64 { 0.5 }
fn yes() -> bool { true }
fn five() -> i64 { 5 }
fn cat_default() -> String { "Feature".to_string() }
fn sev_default() -> String { "medium".to_string() }
impl Default for Triage {
    fn default() -> Self {
        Self {
            authenticity_score: 0.5,
            feasibility_score: 0.5,
            priority_suggestion: 5,
            category_suggestion: "Feature".to_string(),
            severity_suggestion: "medium".to_string(),
            is_duplicate: false,
            duplicate_of: None,
            duplicate_hint: String::new(),
            needs_changes: true,
            no_change_reason: String::new(),
            analysis_summary: String::new(),
            confidence: 0.5,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Understanding {
    #[serde(default)]
    pub problem_type: String,
    #[serde(default)]
    pub restated_requirement: String,
    #[serde(default)]
    pub user_story: Option<String>,
    #[serde(default)]
    pub current_behavior: Option<String>,
    #[serde(default)]
    pub expected_behavior: Option<String>,
    #[serde(default)]
    pub reproduction_steps: Vec<String>,
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuspectedLocation {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RootCause {
    #[serde(default)]
    pub hypothesis: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub suspected_locations: Vec<SuspectedLocation>,
    #[serde(default)]
    pub confidence: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AffectedFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub change_type: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub affected_modules: Vec<String>,
    #[serde(default)]
    pub affected_files: Vec<AffectedFile>,
    #[serde(default)]
    pub related_files: Vec<String>,
    #[serde(default)]
    pub entry_points: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    #[serde(default)]
    pub blast_radius: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanStep {
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Alternative {
    #[serde(default)]
    pub option: String,
    #[serde(default)]
    pub tradeoff: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataModelChange {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImplementationPlan {
    #[serde(default)]
    pub approach: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    #[serde(default)]
    pub data_model_changes: Vec<DataModelChange>,
    #[serde(default)]
    pub new_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub statement: String,
    #[serde(default)]
    pub verify: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestPlan {
    #[serde(default)]
    pub unit: Vec<String>,
    #[serde(default)]
    pub integration: Vec<String>,
    #[serde(default)]
    pub manual: Vec<String>,
    #[serde(default)]
    pub edge_cases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Risk {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub mitigation: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Constraints {
    #[serde(default)]
    pub must: Vec<String>,
    #[serde(default)]
    pub must_not: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Estimate {
    #[serde(default)]
    pub complexity: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeBrief {
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub r#do: Vec<String>,
    #[serde(default)]
    pub dont: Vec<String>,
    #[serde(default)]
    pub files_to_touch: Vec<String>,
    #[serde(default)]
    pub definition_of_done: Vec<String>,
}

fn schema_version_default() -> String { "1.0".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAnalysisSpec {
    #[serde(default = "schema_version_default")]
    pub schema_version: String,
    #[serde(default)]
    pub triage: Triage,
    #[serde(default)]
    pub understanding: Understanding,
    #[serde(default)]
    pub root_cause: Option<RootCause>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub implementation_plan: ImplementationPlan,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub test_plan: TestPlan,
    #[serde(default)]
    pub risks: Vec<Risk>,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub estimate: Option<Estimate>,
    #[serde(default)]
    pub claude_code_brief: ClaudeCodeBrief,
}

impl Default for IssueAnalysisSpec {
    fn default() -> Self {
        Self {
            schema_version: "1.0".to_string(),
            triage: Triage::default(),
            understanding: Understanding::default(),
            root_cause: None,
            scope: Scope::default(),
            implementation_plan: ImplementationPlan::default(),
            acceptance_criteria: vec![],
            test_plan: TestPlan::default(),
            risks: vec![],
            constraints: Constraints::default(),
            assumptions: vec![],
            open_questions: vec![],
            estimate: None,
            claude_code_brief: ClaudeCodeBrief::default(),
        }
    }
}

/// Flattened triage view kept for backward-compatible DB columns and downstream code.
/// `spec` carries the full structured analysis; `analysis_json` is its normalized JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub authenticity_score: f64,
    pub feasibility_score: f64,
    pub priority_suggestion: i64,
    pub category_suggestion: String,
    pub severity_suggestion: String,
    pub affected_modules: Vec<String>,
    pub analysis_summary: String,
    pub is_duplicate: bool,
    pub duplicate_hint: String,
    pub raw_output: String,
    pub analysis_json: String,
    pub spec: IssueAnalysisSpec,
    /// 本次分析的 trace_id（若走自定义 LLM agent），供 dual-write 到 agent_outputs 链回下钻。
    #[serde(default)]
    pub trace_id: Option<String>,
}

impl Default for AnalysisResult {
    fn default() -> Self {
        let spec = IssueAnalysisSpec::default();
        Self {
            authenticity_score: 0.5,
            feasibility_score: 0.5,
            priority_suggestion: 5,
            category_suggestion: "Feature".to_string(),
            severity_suggestion: "medium".to_string(),
            affected_modules: vec![],
            analysis_summary: "自动分析失败，使用默认值".to_string(),
            is_duplicate: false,
            duplicate_hint: String::new(),
            raw_output: String::new(),
            analysis_json: serde_json::to_string(&spec).unwrap_or_else(|_| "{}".to_string()),
            spec,
            trace_id: None,
        }
    }
}

impl AnalysisResult {
    /// Build the flattened result from a parsed spec, clamping numeric ranges.
    fn from_spec(mut spec: IssueAnalysisSpec) -> Self {
        spec.triage.authenticity_score = spec.triage.authenticity_score.clamp(0.0, 1.0);
        spec.triage.feasibility_score = spec.triage.feasibility_score.clamp(0.0, 1.0);
        spec.triage.priority_suggestion = spec.triage.priority_suggestion.clamp(1, 10);
        spec.triage.confidence = spec.triage.confidence.clamp(0.0, 1.0);
        let analysis_json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".to_string());
        Self {
            authenticity_score: spec.triage.authenticity_score,
            feasibility_score: spec.triage.feasibility_score,
            priority_suggestion: spec.triage.priority_suggestion,
            category_suggestion: spec.triage.category_suggestion.clone(),
            severity_suggestion: spec.triage.severity_suggestion.clone(),
            affected_modules: spec.scope.affected_modules.clone(),
            analysis_summary: spec.triage.analysis_summary.clone(),
            is_duplicate: spec.triage.is_duplicate,
            duplicate_hint: spec.triage.duplicate_hint.clone(),
            raw_output: String::new(),
            analysis_json,
            spec,
            trace_id: None,
        }
    }
}

#[allow(dead_code)]
pub fn check_safety(text: &str) -> bool {
    !security::has_obvious_injection(text)
}

/// Read key files from a local repository to build a project context string
/// that enriches the analysis prompt. Gracefully degrades if any read fails.
/// Read all per-project specs from `.autoforge/specs/*.md` — the single source
/// of truth for a project's standards (managed in 项目管理, written by specs.rs).
/// Returns concatenated markdown (each file under a `### <file>` header), capped
/// per file to keep prompts bounded. Empty string when the dir is absent/empty.
pub async fn read_autoforge_specs(repo_path: &str) -> String {
    use tokio::fs;

    let dir = std::path::Path::new(repo_path).join(".autoforge").join("specs");
    let Ok(mut rd) = fs::read_dir(&dir).await else {
        return String::new();
    };

    // Collect .md filenames first so output is deterministic (dir order is not).
    let mut names: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_ascii_lowercase().ends_with(".md") {
            names.push(name);
        }
    }
    names.sort();

    let mut parts: Vec<String> = Vec::new();
    for name in names {
        if let Ok(content) = fs::read_to_string(dir.join(&name)).await {
            let trimmed: String = content.chars().take(6000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("### {}\n{}", name, trimmed));
            }
        }
    }
    parts.join("\n\n")
}

pub async fn build_project_context(repo_path: &str) -> String {
    use tokio::fs;

    let mut parts: Vec<String> = Vec::new();

    // README
    for name in &["README.md", "README.zh.md", "README.en.md", "README.txt", "README"] {
        let path = format!("{}/{}", repo_path, name);
        if let Ok(content) = fs::read_to_string(&path).await {
            let trimmed: String = content.chars().take(3000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("## {}\n{}", name, trimmed));
                break;
            }
        }
    }

    // CLAUDE.md — project-level AI guidance / locked constraints
    if let Ok(content) = fs::read_to_string(format!("{}/CLAUDE.md", repo_path)).await {
        let trimmed: String = content.chars().take(4000).collect();
        if !trimmed.trim().is_empty() {
            parts.push(format!("## CLAUDE.md\n{}", trimmed));
        }
    }

    // 项目规范（.autoforge/specs/*.md）—— 唯一规范来源，约束 scope 与 constraints
    let specs = read_autoforge_specs(repo_path).await;
    if !specs.trim().is_empty() {
        parts.push(format!("## 项目规范（.autoforge/specs）\n{}", specs));
    }

    // Primary manifest/config file
    for name in &["package.json", "Cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle"] {
        let path = format!("{}/{}", repo_path, name);
        if let Ok(content) = fs::read_to_string(&path).await {
            let trimmed: String = content.chars().take(1000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("## {}\n```\n{}\n```", name, trimmed));
                break;
            }
        }
    }

    // Directory tree (depth 2, skip hidden / build dirs). Pure-Rust walk so it
    // works identically on Linux/macOS/Windows (no `find` shell-out — Windows'
    // `find.exe` is an unrelated text-search tool).
    {
        let tree = list_dirs_depth2(repo_path);
        let limited: String = tree.chars().take(1500).collect();
        if !limited.trim().is_empty() {
            parts.push(format!("## 目录结构\n{}", limited));
        }
    }

    // Recent git log
    if let Ok(out) = Command::new("git")
        .arg("-C").arg(repo_path)
        .arg("log")
        .arg("--oneline")
        .arg("-15")
        .output()
        .await
    {
        let log = String::from_utf8_lossy(&out.stdout);
        if !log.trim().is_empty() {
            parts.push(format!("## 近期提交记录\n{}", log));
        }
    }

    parts.join("\n\n")
}

/// List directories up to depth 2 under `root`, skipping hidden and common build
/// dirs. Replaces a `find` shell-out for cross-platform parity. Output mirrors
/// `find`'s newline-separated paths, sorted for determinism.
fn list_dirs_depth2(root: &str) -> String {
    fn skip(name: &str) -> bool {
        name.starts_with('.') || matches!(name, "node_modules" | "target" | "__pycache__")
    }
    let mut dirs: Vec<String> = Vec::new();
    let root_path = std::path::Path::new(root);
    let read = |p: &std::path::Path| -> Vec<std::path::PathBuf> {
        let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| !skip(n))
                    .unwrap_or(false)
            })
            .collect();
        out.sort();
        out
    };
    for d1 in read(root_path) {
        dirs.push(d1.to_string_lossy().to_string());
        for d2 in read(&d1) {
            dirs.push(d2.to_string_lossy().to_string());
        }
    }
    dirs.join("\n")
}

/// Resolve the agent holding the `analysis` forge role (comma-separated field).
/// Returns the first enabled match by creation order, or None if unassigned.
async fn resolve_analysis_agent(db: &crate::db::Db) -> Option<crate::models::agent::Agent> {
    sqlx::query_as::<_, crate::models::agent::Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(forge_role, '') || ',') LIKE '%,analysis,%'
           AND enabled=1
         ORDER BY created_at
         LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

pub async fn analyze(
    db: &crate::db::Db,
    title: &str,
    description: &str,
    project_context: Option<&str>,
    project_id: Option<&str>,
    recalled: Option<&str>,
) -> Result<AnalysisResult> {
    let prompt = match project_context.filter(|c| !c.trim().is_empty()) {
        Some(ctx) => format!(
            "以下是项目的上下文信息，请结合项目实际情况进行分析：\n\n{}\n\n---\n\n请分析以下需求：\n\n标题：{}\n\n描述：{}\n\n请按照要求的 JSON 格式输出分析结果。",
            ctx, title, description
        ),
        None => format!(
            "请分析以下需求：\n\n标题：{}\n\n描述：{}\n\n请按照要求的 JSON 格式输出分析结果。",
            title, description
        ),
    };

    // Run via the analysis Agent's bound LLM (custom/低成本 model) so only Claude Code
    // hits the Claude CLI. Fall back to the local Claude CLI only when no analysis Agent
    // is assigned, so a fresh install still works.
    let (raw, trace_id) = match resolve_analysis_agent(db).await {
        Some(agent) => {
            let mut sys = crate::agents::roles::compose_system_prompt(
                Some("analysis"),
                &agent.prompt_mode,
                &agent.system_prompt,
            );
            // Inject the traced Innate recall under the shared heading so the
            // analysis Agent reasons with past experience — and the Trace page's
            // INNATE flag lights up for analysis too (same marker as run_system_role_text).
            if let Some(r) = recalled.map(str::trim).filter(|s| !s.is_empty()) {
                sys = format!(
                    "{}\n\n## {}\n{}",
                    sys,
                    crate::agents::llm::INNATE_RECALL_HEADING,
                    r
                );
            }
            // 走工具循环：分析 Agent 可按需读取/检索项目真实源码（绑定项目时），
            // 让评估基于实际代码而非仅需求文本。未开启工具/无项目时自动回退单轮。
            let ctx = crate::agents::tools::ToolContext::resolve(db, project_id).await;
            let registry = crate::agents::tools::build_registry_for_agent(db, &agent, &ctx).await;
            // 包一层 scope_run（内部 LLM 调用复用同一 trace），以便取到 trace_id 链回下钻。
            crate::core::trace::scope_run(db, &agent, async {
                let raw = crate::agents::llm::run_agent_text_with_tools(
                    db, &agent, &prompt, Some(&sys), &[], &registry,
                )
                .await?;
                let tid = crate::core::trace::current_trace_id();
                Ok::<_, anyhow::Error>((raw, tid))
            })
            .await?
        }
        None => {
            let sys = match recalled.map(str::trim).filter(|s| !s.is_empty()) {
                Some(r) => format!(
                    "{}\n\n## {}\n{}",
                    SYSTEM_PROMPT,
                    crate::agents::llm::INNATE_RECALL_HEADING,
                    r
                ),
                None => SYSTEM_PROMPT.to_string(),
            };
            (local_claude::run_text(&prompt, Some(&sys)).await?, None)
        }
    };

    // Parse into the full structured spec (with legacy fallback); keep raw for audit.
    let mut result = parse_analysis(&raw).unwrap_or_default();
    result.raw_output = raw;
    result.trace_id = trace_id;
    Ok(result)
}

/// Slice out the outermost `{...}` JSON object from arbitrary LLM output.
fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if start > end {
        return None;
    }
    Some(&text[start..=end])
}

/// Deserialize the full v1.0 spec; fall back to the legacy flat triage object so
/// older model behaviour / cached outputs still parse.
pub fn parse_spec(text: &str) -> Option<IssueAnalysisSpec> {
    let json_str = extract_json(text)?;

    // Preferred: full schema (has a `triage` object).
    if let Ok(spec) = serde_json::from_str::<IssueAnalysisSpec>(json_str) {
        // A bare flat object also deserializes (all fields default), so only accept
        // when it actually carried the new shape.
        if json_has_key(json_str, "triage") || json_has_key(json_str, "claude_code_brief") {
            return Some(spec);
        }
    }

    // Legacy fallback: flat { authenticity_score, ... } → lift into triage/scope.
    #[derive(Deserialize)]
    struct LegacyFlat {
        authenticity_score: Option<f64>,
        feasibility_score: Option<f64>,
        priority_suggestion: Option<i64>,
        category_suggestion: Option<String>,
        severity_suggestion: Option<String>,
        affected_modules: Option<Vec<String>>,
        analysis_summary: Option<String>,
        is_duplicate: Option<bool>,
        duplicate_hint: Option<String>,
    }
    let l: LegacyFlat = serde_json::from_str(json_str).ok()?;
    let mut spec = IssueAnalysisSpec::default();
    spec.triage = Triage {
        authenticity_score: l.authenticity_score.unwrap_or(0.5),
        feasibility_score: l.feasibility_score.unwrap_or(0.5),
        priority_suggestion: l.priority_suggestion.unwrap_or(5),
        category_suggestion: l.category_suggestion.unwrap_or_else(|| "Feature".to_string()),
        severity_suggestion: l.severity_suggestion.unwrap_or_else(|| "medium".to_string()),
        is_duplicate: l.is_duplicate.unwrap_or(false),
        duplicate_of: None,
        duplicate_hint: l.duplicate_hint.unwrap_or_default(),
        needs_changes: true,
        no_change_reason: String::new(),
        analysis_summary: l.analysis_summary.unwrap_or_default(),
        confidence: 0.5,
    };
    spec.scope.affected_modules = l.affected_modules.unwrap_or_default();
    Some(spec)
}

fn json_has_key(json_str: &str, key: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json_str)
        .ok()
        .and_then(|v| v.get(key).cloned())
        .is_some()
}

fn parse_analysis(text: &str) -> Option<AnalysisResult> {
    parse_spec(text).map(AnalysisResult::from_spec)
}
