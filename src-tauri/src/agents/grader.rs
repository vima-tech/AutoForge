use crate::agents::schema::{self, StructuredSchema};
use crate::core::git::GitProxy;
use crate::db::Db;
use crate::models::project::Project;
use serde::{Deserialize, Serialize};

/// §7 risk grade for a change request's diff.
/// 兼作 [`StructuredSchema`]（role=grader）落统一表 `agent_outputs`，与 analysis/test/triage/
/// proposer 同库可串成流水线、支撑字段级体检与版本对比。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grade {
    /// T0 | T1 | T2 | T3（就高不就低）。
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub rationale: String,
    /// 改动类别（按受影响目录/文件归类），门控降级按 class 累积信任。
    #[serde(default)]
    pub change_class: String,
}

fn default_tier() -> String {
    "T3".to_string()
}

impl Default for Grade {
    fn default() -> Self {
        Self {
            tier: default_tier(),
            score: 0,
            rationale: String::new(),
            change_class: String::new(),
        }
    }
}

const GRADER_SCHEMA_TEMPLATE: &str = r#"{
  "tier": "<T0|T1|T2|T3，就高不就低>",
  "score": <0-100 风险分>,
  "rationale": "<判级理由：影响面/可逆性/对数据与安全的触及>",
  "change_class": "<改动类别，如 src/migrations/frontend>"
}"#;

impl StructuredSchema for Grade {
    const ROLE: &'static str = "grader";
    const VERSION: &'static str = "1.0";
    fn schema_template() -> &'static str {
        GRADER_SCHEMA_TEMPLATE
    }
}

/// 把一次风险评级的结构化产出落 `agent_outputs`（role=grader, target=cr）。best-effort。
pub async fn record_to_outputs(db: &Db, cr_id: &str, project_id: &str, grade: &Grade) {
    let output_json = serde_json::to_string(grade).unwrap_or_else(|_| "{}".to_string());
    schema::record(
        db,
        Grade::ROLE,
        Grade::VERSION,
        "cr",
        cr_id,
        Some(project_id),
        None,
        "ok",
        &output_json,
        &grade.rationale,
    )
    .await;
}

pub async fn grade(
    db: &Db,
    project: &Project,
    worktree_path: &str,
    test_passed: Option<bool>,
    iteration: i64,
) -> Grade {
    let git = GitProxy::new(worktree_path);
    let diff = git
        .run(&["diff", &project.branch_dev])
        .await
        .map(|(_, o, _)| o)
        .unwrap_or_default();

    let mut g = heuristic_grade(&diff, test_passed, iteration);
    // LLM can only RAISE the tier (defense against injection-driven downgrade).
    if let Some(llm_tier) = llm_tier(db, &project.id, &diff).await {
        if tier_rank(&llm_tier) > tier_rank(&g.tier) {
            g.rationale = format!("{}；LLM 提级至 {}", g.rationale, llm_tier);
            g.tier = llm_tier;
        }
    }
    g
}

fn tier_rank(t: &str) -> u8 {
    match t {
        "T3" => 3,
        "T2" => 2,
        "T1" => 1,
        _ => 0,
    }
}

fn max_tier(a: &str, b: &str) -> String {
    if tier_rank(a) >= tier_rank(b) {
        a.to_string()
    } else {
        b.to_string()
    }
}

fn heuristic_grade(diff: &str, test_passed: Option<bool>, iteration: i64) -> Grade {
    let files = changed_files(diff);
    let (added, removed) = line_counts(diff);
    let churn = added + removed;

    // ── Hard floor: certain change classes are never below T2/T3 ──
    let floors: &[(&str, &str)] = &[
        ("migrations/", "schema"),
        ("schema.sql", "schema"),
        ("/auth", "auth"),
        ("auth/", "auth"),
        ("payment", "payment"),
        ("billing", "billing"),
        (".env", "secret"),
        ("Cargo.toml", "deps"),
        ("package.json", "deps"),
        ("requirements.txt", "deps"),
        ("go.mod", "deps"),
        ("Gemfile", "deps"),
        ("pom.xml", "deps"),
    ];
    for f in &files {
        let lower = f.to_ascii_lowercase();
        for (needle, class) in floors {
            if lower.contains(needle) {
                return Grade {
                    tier: "T3".to_string(),
                    score: 90,
                    rationale: format!("命中硬地板路径「{}」({})", f, class),
                    change_class: (*class).to_string(),
                };
            }
        }
    }

    let class = primary_class(&files);

    // ── Trivial: docs / tests / comments only ──
    if !files.is_empty() && files.iter().all(|f| is_doc_or_test(f)) {
        let mut g = Grade {
            tier: "T0".to_string(),
            score: 5,
            rationale: format!("仅文档/测试改动（{} 文件，{} 行）", files.len(), churn),
            change_class: class,
        };
        bump_for_signals(&mut g, test_passed, iteration);
        return g;
    }

    // ── Low risk: small localized change ──
    let tier = if churn <= 30 && files.len() <= 3 {
        "T1"
    } else {
        "T2"
    };
    let mut g = Grade {
        tier: tier.to_string(),
        score: if tier == "T1" { 25 } else { 50 },
        rationale: format!("{} 文件，{} 行改动", files.len(), churn),
        change_class: class,
    };
    bump_for_signals(&mut g, test_passed, iteration);
    g
}

/// Failing tests or repeated iterations can only push the tier UP.
fn bump_for_signals(g: &mut Grade, test_passed: Option<bool>, iteration: i64) {
    if iteration >= 3 {
        g.tier = max_tier(&g.tier, "T2");
        g.rationale.push_str("；已迭代≥3轮");
    }
    if test_passed == Some(false) {
        g.tier = max_tier(&g.tier, "T2");
        g.rationale.push_str("；测试未通过");
    }
}

fn changed_files(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            files.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("diff --git a/") {
            if let Some(path) = rest.split(" b/").next() {
                let p = path.trim().to_string();
                if !files.contains(&p) {
                    files.push(p);
                }
            }
        }
    }
    files.retain(|f| f != "/dev/null");
    files.sort();
    files.dedup();
    files
}

fn line_counts(diff: &str) -> (i64, i64) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

fn is_doc_or_test(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md")
        || p.ends_with(".txt")
        || p.contains("/docs/")
        || p.starts_with("docs/")
        || p.contains("/tests/")
        || p.starts_with("tests/")
        || p.contains("_test.")
        || p.contains(".test.")
        || p.contains(".spec.")
        || p.contains("__tests__/")
}

fn primary_class(files: &[String]) -> String {
    let Some(first) = files.first() else {
        return "general".to_string();
    };
    if files.iter().all(|f| is_doc_or_test(f)) {
        return "docs_tests".to_string();
    }
    // top-level directory of the first changed file
    first
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("general")
        .to_string()
}

async fn llm_tier(db: &Db, project_id: &str, diff: &str) -> Option<String> {
    if diff.trim().is_empty() {
        return None;
    }
    let clipped: String = diff.chars().take(10000).collect();
    let prompt = format!(
        "你是代码风险分级器。评估以下 diff 的合并风险，只输出一个等级：\
        \nT0(零风险:文档/格式/纯测试) T1(低:局部小逻辑) T2(中:常规业务) T3(高:schema/迁移/auth/支付/安全/大爆炸半径)。\
        \n只输出 T0/T1/T2/T3 其中之一，不要解释。\n\n```diff\n{clipped}\n```"
    );
    let raw = crate::agents::llm::run_system_role_text(
        db,
        "grader",
        &prompt,
        Some("你是严格的代码风险分级器，只输出 T0/T1/T2/T3。"),
        Some(project_id),
        Some(diff), // 用 diff 作召回键（而非指令前缀），命中本项目同类改动的风险经验
    )
    .await
    .ok()?;
    let up = raw.to_ascii_uppercase();
    for t in ["T3", "T2", "T1", "T0"] {
        if up.contains(t) {
            return Some(t.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // §3.5 防漂移：规范样例必须能解析进 Grade 且关键字段非默认——
    // 改 struct/字段名而忘改 schema 模板（或反之）时此测试会失败。
    #[test]
    fn grade_parses_canonical_example() {
        let ex = r#"{"tier":"T2","score":55,"rationale":"跨多文件功能改动","change_class":"src"}"#;
        let (g, st) = schema::parse_or_default::<Grade>(ex);
        assert_eq!(st, "ok");
        assert_eq!(g.tier, "T2");
        assert_eq!(g.score, 55);
        assert_eq!(g.change_class, "src");
        assert_eq!(<Grade as StructuredSchema>::ROLE, "grader");
    }

    #[test]
    fn grade_default_is_conservative_t3() {
        // 解析失败回退默认：保守按 T3（最高风险，绝不误降级）。
        let (g, st) = schema::parse_or_default::<Grade>("garbage");
        assert_eq!(st, "error");
        assert_eq!(g.tier, "T3");
    }
}
