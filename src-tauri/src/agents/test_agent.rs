//! 测试环节的 schema 驱动 agent（第二个样板，验证 [`super::schema`] 脚手架够通用）。
//! **纯 Rust、零 Tauri 类型**。
//!
//! 设计：机器执行的检查结果是事实权威（verdict/checks/summary 由机器填），LLM 仅在失败时
//! 做结构化诊断与覆盖分析（coverage/failures/designed_cases/…）。两者并入同一 [`TestReport`]
//! schema，落到统一表 `agent_outputs`（role=test），与需求分析产出在同一存储里可串成流水线。

use super::schema::{self, StructuredSchema};
use crate::core::trace::{self, TraceTags};
use crate::db::Db;
use serde::{Deserialize, Serialize};

fn v1() -> String {
    "1.0".to_string()
}

/// 一次合并前测试的结构化报告（schema test v1.0）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    #[serde(default = "v1")]
    pub schema_version: String,
    /// pass | fail | flaky | inconclusive（机器权威）。
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    /// 实际执行的检查及结果（机器填）。
    #[serde(default)]
    pub checks: Vec<CheckOutcome>,
    /// 覆盖分析：与验收标准的映射、缺口、高风险区（LLM 填）。
    #[serde(default)]
    pub coverage: Coverage,
    /// 建议补充的测试用例（LLM 填）。
    #[serde(default)]
    pub designed_cases: Vec<TestCase>,
    /// 每个失败的根因诊断（LLM 填）。
    #[serde(default)]
    pub failures: Vec<FailureDiagnosis>,
    #[serde(default)]
    pub regression_risks: Vec<String>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}

impl Default for TestReport {
    fn default() -> Self {
        Self {
            schema_version: "1.0".to_string(),
            verdict: "inconclusive".to_string(),
            summary: String::new(),
            confidence: None,
            checks: vec![],
            coverage: Coverage::default(),
            designed_cases: vec![],
            failures: vec![],
            regression_risks: vec![],
            recommendations: vec![],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckOutcome {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub output_excerpt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Coverage {
    #[serde(default)]
    pub acceptance_covered: Vec<String>,
    #[serde(default)]
    pub acceptance_uncovered: Vec<String>,
    #[serde(default)]
    pub gaps: Vec<String>,
    #[serde(default)]
    pub risk_areas: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestCase {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub preconditions: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub expected: String,
    #[serde(default)]
    pub priority: String,
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
pub struct FailureDiagnosis {
    #[serde(default)]
    pub check: String,
    #[serde(default)]
    pub root_cause: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub expected_vs_actual: Option<String>,
    #[serde(default)]
    pub fix_direction: Option<String>,
    #[serde(default)]
    pub suspected_locations: Vec<SuspectedLocation>,
}

const SCHEMA_TEMPLATE: &str = r#"{
  "schema_version": "1.0",
  "verdict": "<pass|fail|flaky|inconclusive>",
  "summary": "<一句话结论>",
  "confidence": <0.0-1.0 或 null>,
  "coverage": {
    "acceptance_covered": ["<已被检查覆盖的验收标准 id/描述>"],
    "acceptance_uncovered": ["<未覆盖的验收标准>"],
    "gaps": ["<测试缺口：未覆盖的关键路径/边界>"],
    "risk_areas": ["<高风险区域，建议加强>"]
  },
  "designed_cases": [{
    "name": "<用例名>", "preconditions": <前置或 null>, "input": <输入或 null>,
    "steps": ["<操作步骤>"], "expected": "<预期结果>", "priority": "<high|medium|low>"
  }],
  "failures": [{
    "check": "<失败的检查名>", "root_cause": "<最可能根因>", "evidence": ["<证据：日志/退出码要点>"],
    "expected_vs_actual": <期望 vs 实际或 null>, "fix_direction": <修复方向或 null>,
    "suspected_locations": [{"file": "<相对路径>", "symbol": <函数/组件或 null>, "reason": "<为何怀疑>"}]
  }]
,
  "regression_risks": ["<本次改动潜在的回归风险>"],
  "recommendations": ["<后续测试/工程建议>"]
}"#;

impl StructuredSchema for TestReport {
    const ROLE: &'static str = "test";
    const VERSION: &'static str = "1.0";
    fn schema_template() -> &'static str {
        SCHEMA_TEMPLATE
    }
}

/// 测试 agent 的输入：一次检查的执行结果（与 `tasks::testing::CheckResult` 解耦，避免 agents→tasks 反向依赖）。
pub struct CheckInput {
    pub name: String,
    pub command: String,
    pub ok: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn excerpt(stderr: &str, stdout: &str) -> String {
    let src = if !stderr.trim().is_empty() { stderr } else { stdout };
    src.chars().take(1200).collect()
}

/// 从机器执行结果构建权威基线报告（不含 LLM 分析字段）。
fn baseline(checks: &[CheckInput]) -> TestReport {
    let outcomes: Vec<CheckOutcome> = checks
        .iter()
        .map(|c| CheckOutcome {
            name: c.name.clone(),
            command: c.command.clone(),
            ok: c.ok,
            code: c.code,
            output_excerpt: excerpt(&c.stderr, &c.stdout),
        })
        .collect();
    let failed = checks.iter().filter(|c| !c.ok).count();
    let (verdict, summary) = if checks.is_empty() {
        ("inconclusive".to_string(), "未配置测试命令".to_string())
    } else if failed == 0 {
        ("pass".to_string(), format!("{} 项检查全部通过", checks.len()))
    } else {
        ("fail".to_string(), format!("{} / {} 项检查失败", failed, checks.len()))
    };
    TestReport {
        verdict,
        summary,
        checks: outcomes,
        ..TestReport::default()
    }
}

/// 解析 forge_role 含 `test` 且 enabled 的首个 agent（按创建序）。
async fn resolve_test_agent(db: &Db) -> Option<crate::models::agent::Agent> {
    let holder = sqlx::query_as::<_, crate::models::agent::Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(forge_role, '') || ',') LIKE '%,test,%'
           AND enabled=1
         ORDER BY created_at
         LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;
    // 未绑定 LLM 时回落全局默认 LLM（治本：漏配不致命）。
    Some(crate::agents::llm::with_default_llm(db, holder).await)
}

/// 产出并落库一次测试的结构化报告。**best-effort**：失败只记日志，绝不阻断测试闸口。
/// 机器结果始终落库；仅当存在失败且配置了 test agent 时，附加 LLM 结构化诊断。
pub async fn report(
    db: &Db,
    cr_id: &str,
    cr_title: &str,
    project_id: &str,
    checks: &[CheckInput],
    acceptance_json: Option<&str>,
) {
    let mut rep = baseline(checks);
    let mut status: &str = "ok";
    let mut raw = String::new();
    let mut trace_id: Option<String> = None;

    let has_failure = checks.iter().any(|c| !c.ok);
    if has_failure {
        if let Some(agent) = resolve_test_agent(db).await {
            match enrich_with_llm(db, &agent, cr_title, project_id, checks, acceptance_json).await {
                Ok((parsed, st, tid, raw_out)) => {
                    // LLM 仅补充分析字段；事实字段（verdict/checks/summary）保持机器权威。
                    rep.coverage = parsed.coverage;
                    rep.designed_cases = parsed.designed_cases;
                    rep.failures = parsed.failures;
                    rep.regression_risks = parsed.regression_risks;
                    rep.recommendations = parsed.recommendations;
                    rep.confidence = parsed.confidence;
                    status = st;
                    trace_id = tid;
                    raw = raw_out;
                }
                Err(e) => {
                    status = "partial"; // 机器结果有效，LLM 诊断缺失
                    eprintln!("[test_agent] LLM 诊断失败（已忽略，仅记录机器结果）：{}", e);
                }
            }
        }
    }

    let output_json = serde_json::to_string(&rep).unwrap_or_else(|_| "{}".to_string());
    schema::record(
        db,
        TestReport::ROLE,
        TestReport::VERSION,
        "cr",
        cr_id,
        Some(project_id),
        trace_id.as_deref(),
        status,
        &output_json,
        &raw,
    )
    .await;
}

/// 调用 test agent 的 LLM 产出结构化诊断；捕获 trace_id 以便下钻。
async fn enrich_with_llm(
    db: &Db,
    agent: &crate::models::agent::Agent,
    cr_title: &str,
    project_id: &str,
    checks: &[CheckInput],
    acceptance_json: Option<&str>,
) -> anyhow::Result<(TestReport, &'static str, Option<String>, String)> {
    let sys = crate::agents::roles::compose_system_prompt(
        Some("test"),
        &agent.prompt_mode,
        &agent.system_prompt,
    );

    let checks_block = checks
        .iter()
        .map(|c| {
            format!(
                "### {} （{}）\n命令：{}\n退出码：{}\n输出摘要：\n{}",
                c.name,
                if c.ok { "通过" } else { "失败" },
                c.command,
                c.code,
                excerpt(&c.stderr, &c.stdout),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let acc = acceptance_json
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("（无）");

    let prompt = format!(
        "以下是一次合并前测试的执行结果，请据此产出结构化测试报告。\n\n\
         ## 变更请求\n{title}\n\n\
         ## 验收标准\n{acc}\n\n\
         ## 已执行检查与结果\n{checks}\n\n\
         {contract}",
        title = cr_title,
        acc = acc,
        checks = checks_block,
        contract = TestReport::prompt_contract(),
    );

    let ctx = crate::agents::tools::ToolContext::resolve(db, Some(project_id)).await;
    let registry = crate::agents::tools::build_registry_for_agent(db, agent, &ctx).await;

    // 设标签 + 包一层 scope_run，使内部 LLM 调用复用同一 trace，且我们能取到 trace_id。
    let tags = TraceTags {
        project_id: Some(project_id.to_string()),
        ..Default::default()
    };
    let agent_cl = agent.clone();
    let (raw, tid) = trace::with_tags(tags, async move {
        trace::scope_run(db, &agent_cl, async {
            let raw = crate::agents::llm::run_agent_text_with_tools(
                db, &agent_cl, &prompt, Some(&sys), &[], &registry,
            )
            .await?;
            let tid = trace::current_trace_id();
            Ok::<_, anyhow::Error>((raw, tid))
        })
        .await
    })
    .await?;

    let (parsed, st) = schema::parse_or_default::<TestReport>(&raw);
    Ok((parsed, st, tid, raw))
}
