use crate::core::{event, git::GitProxy, security};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use uuid::Uuid;

/// Node 07 — Security Audit Agent (design §4.3).
/// Scans the merged diff for secret leakage, dangerous operations and injection
/// patterns. Runs a cheap heuristic pass (always) plus an optional LLM audit
/// (best-effort; degrades to heuristic-only when no security agent is configured).
/// High/critical findings are filed back as issues, re-entering the pipeline loop.
pub async fn run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    let cr = sqlx::query_as::<_, crate::models::change_request::ChangeRequest>(
        "SELECT * FROM change_requests WHERE id=?",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| anyhow!("cr {} not found", cr_id))?;

    let project =
        sqlx::query_as::<_, crate::models::project::Project>("SELECT * FROM projects WHERE id=?")
            .bind(&cr.project_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("project {} not found", cr.project_id))?;

    let audit_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO security_audits (id, project_id, change_request_id, status)
         VALUES (?, ?, ?, 'running')",
    )
    .bind(&audit_id)
    .bind(&project.id)
    .bind(cr_id)
    .execute(db)
    .await?;

    let diff = load_diff(db, &project.repo_path, cr_id).await;
    let mut findings = heuristic_scan(&diff);

    // Optional LLM deep audit — best effort, never blocks the pipeline.
    if !diff.trim().is_empty() {
        if let Some(extra) = llm_audit(db, &cr.project_id, &diff).await {
            findings.extend(extra);
        }
    }

    let severity = max_severity(&findings);
    let status = match severity.as_str() {
        "high" | "critical" => "failed",
        "medium" | "low" => "flagged",
        _ => "passed",
    };
    let summary = if findings.is_empty() {
        "未发现安全问题".to_string()
    } else {
        format!("发现 {} 项安全问题（最高 {}）", findings.len(), severity)
    };

    // File high/critical findings back into the pipeline as a Security issue.
    let mut issues_created: Vec<String> = Vec::new();
    if matches!(severity.as_str(), "high" | "critical") {
        let title = format!("安全审查发现高危问题：{}", cr_id);
        let body = findings
            .iter()
            .filter(|f| matches!(f.severity.as_str(), "high" | "critical"))
            .map(|f| format!("[{}] {} — {}", f.severity, f.title, f.detail))
            .collect::<Vec<_>>()
            .join("\n");
        let issue_id = Uuid::new_v4().to_string();
        let fp = security::fingerprint(&title, &body);
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO issues
             (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'security_audit', ?, ?, 'Security', ?, 9, 'pending_analysis', ?)",
        )
        .bind(&issue_id)
        .bind(&project.id)
        .bind(&title)
        .bind(&body)
        .bind(&severity)
        .bind(&fp)
        .execute(db)
        .await?;
        if inserted.rows_affected() > 0 {
            issues_created.push(issue_id.clone());
            let _ = enqueue(
                db,
                tx,
                "analysis",
                &format!("analysis:{}", issue_id),
                JobPayload::Analysis { issue_id },
            )
            .await;
        }
    }

    let findings_json = json!(findings
        .iter()
        .map(|f| json!({"severity": f.severity, "title": f.title, "detail": f.detail}))
        .collect::<Vec<_>>())
    .to_string();

    sqlx::query(
        "UPDATE security_audits
         SET status=?, severity=?, summary=?, findings_json=?, issues_created=?, completed_at=datetime('now')
         WHERE id=?",
    )
    .bind(status)
    .bind(&severity)
    .bind(&summary)
    .bind(&findings_json)
    .bind(serde_json::to_string(&issues_created)?)
    .bind(&audit_id)
    .execute(db)
    .await?;

    // 结构化产出落统一表（role=security），best-effort：失败只记日志不阻断。
    {
        use crate::agents::schema::{self, StructuredSchema};
        let report = SecurityReport {
            verdict: status.to_string(),
            severity: if findings.is_empty() { "none".to_string() } else { severity.clone() },
            summary: summary.clone(),
            findings: findings.clone(),
        };
        let output_json = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
        schema::record(
            db,
            SecurityReport::ROLE,
            SecurityReport::VERSION,
            "cr",
            cr_id,
            Some(&project.id),
            None,
            "ok",
            &output_json,
            "",
        )
        .await;
    }

    if matches!(severity.as_str(), "high" | "critical") {
        crate::core::notify::dispatch(db, "security_high", "安全审查发现高危问题", &summary).await;
    }

    event::emit(
        app,
        event::AppEvent::SecurityAuditCompleted {
            cr_id: cr_id.to_string(),
            audit_id,
            severity,
            summary,
        },
    );

    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Finding {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    detail: String,
}

/// 一次合并后安全审查的结构化报告（schema security v1.0）。落统一表 `agent_outputs`
/// （role=security, target=cr），与 analysis/test/triage/proposer/grader 同库串流水线。
/// 事实字段（verdict/severity）由启发式 + LLM 合并的 findings 推出（机器权威）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SecurityReport {
    /// passed | flagged | failed（按最高严重度映射）。
    #[serde(default)]
    verdict: String,
    /// none | low | medium | high | critical。
    #[serde(default)]
    severity: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<Finding>,
}

const SECURITY_SCHEMA_TEMPLATE: &str = r#"{
  "verdict": "<passed|flagged|failed>",
  "severity": "<none|low|medium|high|critical>",
  "summary": "<一句话结论>",
  "findings": [{"severity": "<low|medium|high|critical>", "title": "<问题简述>", "detail": "<成因/位置/修复>"}]
}"#;

impl crate::agents::schema::StructuredSchema for SecurityReport {
    const ROLE: &'static str = "security";
    const VERSION: &'static str = "1.0";
    fn schema_template() -> &'static str {
        SECURITY_SCHEMA_TEMPLATE
    }
}

/// Pre-merge deterministic security gate (design §4.3 — shift-left).
///
/// Runs ONLY the cheap, zero-dependency heuristic pass over the CR's worktree
/// diff *before* it lands on dev. Returns `Some(reason)` — a human-readable
/// markdown block — when a high/critical finding (hardcoded secret, `rm -rf /`,
/// `os.system`, `shell=True`, …) must block the merge; `None` to allow.
///
/// The deep LLM audit deliberately stays in the post-merge `run()` (Node 07):
/// the gate must be fast, deterministic and free so it never stalls or bills the
/// pipeline, while the LLM pass keeps providing the richer record + learning loop
/// on what actually merged.
pub(crate) async fn pre_merge_gate(db: &Db, repo_path: &str, cr_id: &str) -> Option<String> {
    let diff = load_diff(db, repo_path, cr_id).await;
    if diff.trim().is_empty() {
        return None;
    }
    let findings = heuristic_scan(&diff);
    let severity = max_severity(&findings);
    if !matches!(severity.as_str(), "high" | "critical") {
        return None;
    }
    let detail = findings
        .iter()
        .filter(|f| matches!(f.severity.as_str(), "high" | "critical"))
        .map(|f| format!("- **[{}]** {} — `{}`", f.severity, f.title, f.detail))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "## 安全门拦截合并\n\n合并前安全扫描发现 {} 项高危问题（最高 **{}**），已阻断合并以防风险代码进入 dev。请在代码中修复后重新执行；如为误报，可调整改动或人工删除该需求。\n\n{}\n",
        findings
            .iter()
            .filter(|f| matches!(f.severity.as_str(), "high" | "critical"))
            .count(),
        severity,
        detail
    ))
}

/// Returns the merged diff for the change request's latest worktree branch.
async fn load_diff(db: &Db, repo_path: &str, cr_id: &str) -> String {
    let session: Option<crate::models::worktree::WorktreeSession> = sqlx::query_as(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let Some(session) = session else {
        return String::new();
    };
    let git = GitProxy::new(repo_path);
    let range = format!("{}^1", session.branch_name);
    match git.run(&["diff", &range, &session.branch_name]).await {
        Ok((_, stdout, _)) => stdout,
        Err(_) => String::new(),
    }
}

static SECRET_PATTERNS: Lazy<Vec<(&'static str, &'static str)>> = Lazy::new(|| {
    vec![
        ("critical", r"(?i)-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ("critical", r"AKIA[0-9A-Z]{16}"),
        ("high", r#"(?i)(api[_-]?key|secret|token|passwd|password)\s*[:=]\s*['""][^'""\s]{8,}['""]"#),
        ("high", r"(?i)aws_secret_access_key\s*[:=]"),
        ("high", r"(?i)ghp_[0-9a-zA-Z]{30,}"),
    ]
});

static DANGER_PATTERNS: Lazy<Vec<(&'static str, &'static str, &'static str)>> = Lazy::new(|| {
    vec![
        ("high", "rm -rf 危险删除", r"rm\s+-rf\s+/"),
        ("high", "shell 注入风险", r"(?i)subprocess\.[a-z]+\([^)]*shell\s*=\s*True"),
        ("high", "os.system 调用", r"os\.system\s*\("),
        ("medium", "eval 动态执行", r"\beval\s*\("),
        ("medium", "exec 动态执行", r"\bexec\s*\("),
        ("medium", "XSS: dangerouslySetInnerHTML", r"dangerouslySetInnerHTML"),
        ("medium", "SQL 拼接风险", r#"(?i)(execute|query)\s*\(\s*[f]?['""].*\+\s*"#),
    ]
});

/// Heuristic scan over **added** diff lines (lines starting with `+`, excluding `+++` headers).
fn heuristic_scan(diff: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let secrets: Vec<(&str, Regex)> = SECRET_PATTERNS
        .iter()
        .filter_map(|(sev, p)| Regex::new(p).ok().map(|r| (*sev, r)))
        .collect();
    let dangers: Vec<(&str, &str, Regex)> = DANGER_PATTERNS
        .iter()
        .filter_map(|(sev, name, p)| Regex::new(p).ok().map(|r| (*sev, *name, r)))
        .collect();

    for line in diff.lines() {
        if !line.starts_with('+') || line.starts_with("+++") {
            continue;
        }
        let added = &line[1..];
        for (sev, re) in &secrets {
            if re.is_match(added) {
                findings.push(Finding {
                    severity: sev.to_string(),
                    title: "疑似密钥/凭证泄露".to_string(),
                    detail: security::safe_truncate(added.trim(), 160),
                });
            }
        }
        for (sev, name, re) in &dangers {
            if re.is_match(added) {
                findings.push(Finding {
                    severity: sev.to_string(),
                    title: name.to_string(),
                    detail: security::safe_truncate(added.trim(), 160),
                });
            }
        }
    }
    findings
}

/// Best-effort LLM audit. Returns None when no security agent is configured or on any error.
async fn llm_audit(db: &Db, project_id: &str, diff: &str) -> Option<Vec<Finding>> {
    let clipped: String = diff.chars().take(12000).collect();
    let prompt = format!(
        "你是安全审计 Agent。审查以下代码 diff，只报告真实安全问题（密钥泄露、注入、越权、危险依赖、不安全反序列化等）。\
        \n严格输出 JSON 数组，每项 {{\"severity\":\"low|medium|high|critical\",\"title\":\"...\",\"detail\":\"...\"}}；无问题输出 []。\n\n```diff\n{clipped}\n```"
    );
    let raw = crate::agents::llm::run_system_role_text(
        db,
        "security",
        &prompt,
        Some("你是严格的安全审计 Agent，只输出 JSON 数组。"),
        Some(project_id),
        Some(diff), // 用 diff 作召回键，命中本项目同类改动的安全经验/误报历史
    )
    .await
    .ok()?;
    let json_slice = extract_json_array(&raw)?;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(json_slice).ok()?;
    let findings = parsed
        .into_iter()
        .filter_map(|v| {
            let sev = v.get("severity")?.as_str()?.to_ascii_lowercase();
            let sev = match sev.as_str() {
                "low" | "medium" | "high" | "critical" => sev,
                _ => "medium".to_string(),
            };
            Some(Finding {
                severity: sev,
                title: v
                    .get("title")
                    .and_then(|s| s.as_str())
                    .unwrap_or("安全问题")
                    .to_string(),
                detail: v
                    .get("detail")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    Some(findings)
}

fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}

fn max_severity(findings: &[Finding]) -> String {
    let rank = |s: &str| match s {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    };
    findings
        .iter()
        .map(|f| f.severity.as_str())
        .max_by_key(|s| rank(s))
        .unwrap_or("none")
        .to_string()
}
