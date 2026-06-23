use crate::core::{event, security};
use crate::db::Db;
use crate::models::job::JobPayload;
use crate::tasks::runner::{enqueue, JobSender};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    test: Option<TestConfig>,
    quality: Option<QualityConfig>,
}

#[derive(Debug, Deserialize)]
struct TestConfig {
    unit: Option<CommandConfig>,
    integration: Option<CommandConfig>,
}

#[derive(Debug, Deserialize)]
struct CommandConfig {
    command: String,
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct QualityConfig {
    lint: Option<String>,
    typing: Option<String>,
    security: Option<String>,
}

pub(crate) struct CheckResult {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) ok: bool,
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Job entry point (kept for the dispatch table). Delegates to `run_and_gate`
/// and discards the pass/fail result.
pub async fn run(db: &Db, tx: &JobSender, app: &tauri::AppHandle, cr_id: &str) -> Result<()> {
    run_and_gate(db, tx, app, cr_id).await.map(|_| ())
}

/// Run the project's configured checks for a change request.
///
/// Checks run against the CR's worktree (the not-yet-merged branch) when one is
/// still on disk, otherwise against the project repo. Returns `Ok(true)` when
/// every check passes (or none are configured) and `Ok(false)` when any fails.
/// On failure a follow-up bug issue and scan finding are recorded so the failure
/// is tracked even though the merge is blocked upstream.
pub async fn run_and_gate(
    db: &Db,
    tx: &JobSender,
    app: &tauri::AppHandle,
    cr_id: &str,
) -> Result<bool> {
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

    // Prefer the CR's worktree (the un-merged branch) so tests gate BEFORE the
    // merge instead of validating dev after the fact.
    let session = sqlx::query_as::<_, crate::models::worktree::WorktreeSession>(
        "SELECT * FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await?;
    let test_path = session
        .as_ref()
        .map(|s| s.worktree_path.clone())
        .filter(|p| std::path::Path::new(p).exists())
        .unwrap_or_else(|| project.repo_path.clone());

    let session_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO test_sessions
         (id, project_id, session_type, change_request_id, trigger, status, started_at)
         VALUES (?, ?, 'reactive', ?, 'pre_merge', 'running', datetime('now'))",
    )
    .bind(&session_id)
    .bind(&project.id)
    .bind(cr_id)
    .execute(db)
    .await?;

    let checks = configured_checks(crate::commands::run_config::effective_config(&project).as_deref());

    // 构建池：占一个全局许可再跑编译/测试，限制跨项目/CR 的并发编译数，避免批量合并时
    // 多个 rustc/tsc 同时把 CPU/内存打满。持有至本 CR 全部 check 跑完后随作用域释放。
    let build_pool = crate::state::build_pool();
    let _build_permit = build_pool.acquire().await;

    let mut results = Vec::new();
    for (name, command, timeout) in checks {
        results.push(run_check(&test_path, name, command, timeout).await);
    }

    // 差量安全门 + 自动供料：security(cargo audit)从「绝对门」改成「回归门」——合并前测试只
    // 看本次改动有没有**新引入**依赖漏洞；项目基线本来就有的（rsa、Tauri GTK3 链等）不阻断
    // 合并，而是作为需求自动供料进传送带（见 demote_preexisting_security）。
    let preexisting_adv =
        demote_preexisting_security(&mut results, &test_path, &project.branch_dev).await;
    for adv in &preexisting_adv {
        let title = format!("[安全] 依赖公告 {}（基线既有，非本次引入）", adv);
        let description = format!(
            "项目依赖树存在安全公告 {}，非本次改动引入，已放行合并。建议评估修复或在 \
             .cargo/audit.toml 登记接受。\nhttps://rustsec.org/advisories/{}",
            adv, adv
        );
        // 稳定 fingerprint（仅取决于 advisory id）+ INSERT OR IGNORE → 跨多次合并自动去重。
        let fp = security::fingerprint("dep-advisory", adv);
        let issue_id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO issues
             (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'security_audit', ?, ?, 'Debt', 'medium', 5, 'pending_analysis', ?)",
        )
        .bind(&issue_id)
        .bind(&project.id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .execute(db)
        .await;
        if matches!(inserted, Ok(ref r) if r.rows_affected() > 0) {
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

    let failed = results.iter().filter(|r| !r.ok).collect::<Vec<_>>();
    let status = if failed.is_empty() {
        "passed"
    } else {
        "failed"
    };
    let summary = if results.is_empty() {
        "未配置测试命令，标记为通过".to_string()
    } else if failed.is_empty() {
        format!("{} 项检查全部通过", results.len())
    } else {
        format!("{} / {} 项检查失败", failed.len(), results.len())
    };

    let results_json = serde_json::json!({
        "checks": results.iter().map(|r| serde_json::json!({
            "name": r.name,
            "command": r.command,
            "ok": r.ok,
            "code": r.code,
            "stdout": r.stdout.chars().take(4000).collect::<String>(),
            "stderr": r.stderr.chars().take(4000).collect::<String>(),
        })).collect::<Vec<_>>()
    })
    .to_string();

    let mut issues_created = Vec::new();
    if !failed.is_empty() {
        let title = format!("合并前测试失败：{}", cr_id);
        let description = failed
            .iter()
            .map(|r| {
                format!(
                    "检查 `{}` 失败。\n\n命令：{}\n退出码：{}\n\nstderr:\n{}",
                    r.name, r.command, r.code, r.stderr
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        let issue_id = Uuid::new_v4().to_string();
        let fp = security::fingerprint(&title, &description);
        sqlx::query(
            "INSERT INTO issues
             (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'scan', ?, ?, 'Bug', 'high', 8, 'pending_analysis', ?)",
        )
        .bind(&issue_id)
        .bind(&project.id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .execute(db)
        .await?;

        let finding_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO scan_findings
             (id, test_session_id, check_type, severity, title, description, fingerprint, issue_entry_id)
             VALUES (?, ?, 'test', 'high', ?, ?, ?, ?)",
        )
        .bind(&finding_id)
        .bind(&session_id)
        .bind(&title)
        .bind(&description)
        .bind(&fp)
        .bind(&issue_id)
        .execute(db)
        .await?;

        issues_created.push(issue_id.clone());
        let _ = enqueue(
            db,
            tx,
            "analysis",
            &format!("analysis:{}", issue_id),
            JobPayload::Analysis {
                issue_id: issue_id.clone(),
            },
        )
        .await;
        event::emit(
            app,
            event::AppEvent::IssueCreated {
                issue_id,
                project_id: project.id.clone(),
            },
        );
    }

    sqlx::query(
        "UPDATE test_sessions
         SET status=?, summary=?, results_json=?, issues_created=?, completed_at=datetime('now')
         WHERE id=?",
    )
    .bind(status)
    .bind(&summary)
    .bind(&results_json)
    .bind(serde_json::to_string(&issues_created)?)
    .bind(&session_id)
    .execute(db)
    .await?;

    if status == "failed" {
        crate::core::notify::dispatch(db, "test_failed", "测试失败", &summary).await;
    }

    event::emit(
        app,
        event::AppEvent::TestCompleted {
            cr_id: cr_id.to_string(),
            test_session_id: session_id,
            status: status.to_string(),
            summary,
        },
    );

    // schema 驱动测试 agent：把本次测试结果（失败时附 LLM 结构化诊断）落到统一产出表
    // agent_outputs（role=test），与需求分析产出同存可串成流水线。best-effort，绝不影响闸口结果。
    {
        let (cr_title, acceptance) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT title, acceptance_json FROM issues WHERE id=?",
        )
        .bind(&cr.issue_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| (cr_id.to_string(), None));

        let check_inputs: Vec<crate::agents::test_agent::CheckInput> = results
            .iter()
            .map(|r| crate::agents::test_agent::CheckInput {
                name: r.name.clone(),
                command: r.command.clone(),
                ok: r.ok,
                code: r.code,
                stdout: r.stdout.clone(),
                stderr: r.stderr.clone(),
            })
            .collect();

        crate::agents::test_agent::report(
            db,
            cr_id,
            &cr_title,
            &project.id,
            &check_inputs,
            acceptance.as_deref(),
        )
        .await;
    }

    Ok(failed.is_empty())
}

/// 差量安全门：把 security(cargo audit)检查从「绝对门」改成「回归门」。
///
/// 只有本 CR 的 Cargo.lock 相对 **dev 基线新引入**的 advisory 才保持失败、阻断合并；项目基线
/// 本来就存在的 advisory（rsa、Tauri 2.x GTK3 链等，几乎都来自传递依赖而非本次改动）不阻断
/// ——它们作为「基线既有」返回给调用方走自动供料登记需求。
///
/// 仅处理 `cargo audit` 命令（其它语言的 security 命令保持原样）。基线取自 **CR 分支与 dev 的
/// 合并基（fork 点）** 的同名 lock；拿不到基线时按用户优先级「不卡合并」放行（视为全部既有）。
/// 返回：被判定为「基线既有」的 advisory id 集（去重排序），供调用方自动供料。
async fn demote_preexisting_security(
    results: &mut [CheckResult],
    worktree: &str,
    branch_dev: &str,
) -> Vec<String> {
    let mut preexisting: Vec<String> = Vec::new();
    for r in results.iter_mut() {
        if r.name != "security" || r.ok || !r.command.contains("cargo audit") {
            continue;
        }
        let lock = audit_file_arg(&r.command).unwrap_or_else(|| "Cargo.lock".to_string());
        // 当前 CR 的 advisory 集（在 worktree 跑，应用其 .cargo/audit.toml 忽略规则）。
        // 跑不出来（缺工具/DB/解析失败）→ 不是「项目本来的漏洞」语义，保留原失败、不误放行。
        let Some(cur) = cargo_audit_vuln_ids(worktree, &lock).await else {
            continue;
        };
        let base = baseline_vuln_ids(worktree, branch_dev, &lock).await;
        let mut new_ids: Vec<String> = cur.iter().filter(|id| !base.contains(*id)).cloned().collect();
        new_ids.sort();
        let mut pre: Vec<String> = cur.iter().filter(|id| base.contains(*id)).cloned().collect();
        pre.sort();
        preexisting.extend(pre.iter().cloned());

        if new_ids.is_empty() {
            // 没有新引入 → 不阻断合并。改写为通过，输出说明（供报告/遥测）。
            r.ok = true;
            r.code = 0;
            r.stdout = format!(
                "差量安全门：本次改动未新引入依赖漏洞，放行合并。\n基线既有 advisory（不阻断，已自动供料登记需求）：{}",
                if pre.is_empty() { "无".into() } else { pre.join(", ") }
            );
            r.stderr.clear();
        } else {
            // 有新引入 → 仍阻断，但只报新增的，避免淹没在基线噪音里。
            r.stderr = format!(
                "本次改动新引入依赖漏洞（阻断合并）：{}\n基线既有问题不计入：{}",
                new_ids.join(", "),
                if pre.is_empty() { "无".into() } else { pre.join(", ") }
            );
        }
    }
    preexisting.sort();
    preexisting.dedup();
    preexisting
}

/// 解析 cargo audit 命令里的 `--file <path>`（lockfile 路径）；无则 None（用默认 Cargo.lock）。
fn audit_file_arg(command: &str) -> Option<String> {
    let toks: Vec<&str> = command.split_whitespace().collect();
    toks.iter()
        .position(|t| *t == "--file")
        .and_then(|i| toks.get(i + 1))
        .map(|s| s.to_string())
}

/// 在 `cwd` 内跑 `cargo audit --no-fetch --file <lock_path> --json`，返回 vulnerability 的
/// advisory id 集（应用 cwd 下 .cargo/audit.toml 的忽略规则）。工具/DB 缺失或解析失败 → None。
async fn cargo_audit_vuln_ids(cwd: &str, lock_path: &str) -> Option<HashSet<String>> {
    let cmd = format!("cargo audit --no-fetch --file {} --json", lock_path);
    let mut c = crate::core::platform::shell(&cmd);
    c.current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = c.output().await.ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let list = v.get("vulnerabilities")?.get("list")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|it| it.get("advisory")?.get("id")?.as_str().map(String::from))
            .collect(),
    )
}

/// 基线 advisory 集：取 **CR 分支与 dev 的合并基（fork 点）** 的同名 lock 写临时文件后在
/// worktree 内审计（同一忽略规则）。
///
/// 用 fork 点而非 dev tip 是关键——「本次改动引入」只能相对 CR 的**起点**算：fork 后 dev 并行
/// 修复（如 quinn 0185 在 dev 上已修，但 CR 从更早的 dev 切出仍带旧版）或 dev 并行引入的问题，
/// 都不该算到本 CR 头上。若 Phase-1 已把 dev 并入 worktree，则 dev 成为 HEAD 祖先、merge-base
/// 自然就是 dev tip，逻辑退化为「只看 CR 在最新 dev 上的净新增」，同样正确。
///
/// 在 worktree 内跑 git（HEAD=CR 分支；worktree 与主仓库共享对象库，可 show 任意 sha）。拿不到
/// 合并基则回退 dev tip，再不行 → 空集（按「不卡合并」放行）。
async fn baseline_vuln_ids(worktree: &str, branch_dev: &str, lock_rel: &str) -> HashSet<String> {
    let git = crate::core::git::GitProxy::new(worktree);
    // fork 点：CR 分支 HEAD 与 dev 的合并基。拿不到则回退 dev tip。
    let base_ref = git
        .run(&["merge-base", "HEAD", branch_dev])
        .await
        .ok()
        .filter(|(c, _, _)| *c == 0)
        .map(|(_, o, _)| o.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| branch_dev.to_string());
    let spec = format!("{}:{}", base_ref, lock_rel);
    let content = match git.run(&["show", &spec]).await {
        Ok((0, out, _)) if !out.trim().is_empty() => out,
        _ => return HashSet::new(),
    };
    let tmp = std::env::temp_dir().join(format!("af-baseline-{}.lock", Uuid::new_v4()));
    if std::fs::write(&tmp, content.as_bytes()).is_err() {
        return HashSet::new();
    }
    let ids = cargo_audit_vuln_ids(worktree, &tmp.to_string_lossy())
        .await
        .unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    ids
}

/// 从某 CR 最近一次合并门测试会话提取失败详情，组装成可读 Markdown 回写到 worktree
/// `report_content`，供审核页「合并失败原因」展示真正有用的测试输出——否则该字段仍是
/// 编码 Agent 的实现摘要（「## 改动摘要」），对诊断合并失败毫无帮助。
///
/// 仅应在测试失败导致 merge_failed / merge_conflict 时调用；找不到失败会话则不动
/// `report_content`（保留原实现摘要）。与安全门 / 落地失败写 `report_content` 的现有
/// 模式一致。best-effort，任何 DB 错误都静默吞掉、不影响闸口结果。
pub(crate) async fn persist_test_failure_report(db: &Db, cr_id: &str) {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT summary, results_json FROM test_sessions
         WHERE change_request_id=? AND trigger='pre_merge' AND status='failed'
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(cr_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let Some((summary, results_json)) = row else {
        return;
    };

    // 取字符串尾部 n 个字符（失败信息通常在编译/测试输出末尾）。
    fn tail_chars(s: &str, n: usize) -> String {
        let mut v: Vec<char> = s.chars().rev().take(n).collect();
        v.reverse();
        v.into_iter().collect()
    }

    let mut body = String::new();
    if let Some(checks) = results_json
        .as_deref()
        .and_then(|rj| serde_json::from_str::<serde_json::Value>(rj).ok())
        .and_then(|v| v.get("checks").and_then(|c| c.as_array()).cloned())
    {
        for c in &checks {
            if c.get("ok").and_then(|b| b.as_bool()).unwrap_or(true) {
                continue;
            }
            let name = c.get("name").and_then(|x| x.as_str()).unwrap_or("check");
            let command = c.get("command").and_then(|x| x.as_str()).unwrap_or("");
            let code = c
                .get("code")
                .and_then(|x| x.as_i64())
                .map(|c| format!("　退出码：{}", c))
                .unwrap_or_default();
            let stderr = c.get("stderr").and_then(|x| x.as_str()).unwrap_or("");
            let stdout = c.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
            let out = if stderr.trim().is_empty() { stdout } else { stderr };
            let tail = tail_chars(out, 2000);
            let tail = if tail.trim().is_empty() {
                "(无输出)".to_string()
            } else {
                tail.trim().to_string()
            };
            body.push_str(&format!(
                "### `{}`\n\n命令：`{}`{}\n\n```\n{}\n```\n\n",
                name, command, code, tail
            ));
        }
    }
    if body.is_empty() {
        body.push_str("（未捕获到具体检查输出，请查看测试会话或执行日志。）\n");
    }
    let report = format!("## 合并前测试失败\n\n{}，已阻断合并。\n\n{}", summary, body);

    let _ = sqlx::query(
        "UPDATE worktree_sessions SET report_content=?
         WHERE id=(SELECT id FROM worktree_sessions WHERE change_request_id=? ORDER BY rowid DESC LIMIT 1)",
    )
    .bind(&report)
    .bind(cr_id)
    .execute(db)
    .await;
}

pub(crate) fn configured_checks(config_yaml: Option<&str>) -> Vec<(String, String, u64)> {
    let Some(raw) = config_yaml else {
        return Vec::new();
    };
    let Ok(config) = serde_yaml::from_str::<ProjectConfig>(raw) else {
        return Vec::new();
    };

    let mut checks = Vec::new();
    if let Some(test) = config.test {
        if let Some(unit) = test.unit {
            checks.push((
                "unit".to_string(),
                unit.command,
                unit.timeout.unwrap_or(120),
            ));
        }
        if let Some(integration) = test.integration {
            checks.push((
                "integration".to_string(),
                integration.command,
                integration.timeout.unwrap_or(300),
            ));
        }
    }
    if let Some(quality) = config.quality {
        if let Some(command) = quality.lint {
            checks.push(("lint".to_string(), command, 120));
        }
        if let Some(command) = quality.typing {
            checks.push(("typing".to_string(), command, 120));
        }
        if let Some(command) = quality.security {
            checks.push(("security".to_string(), command, 120));
        }
    }
    checks
}

pub(crate) async fn run_check(
    repo_path: &str,
    name: String,
    command: String,
    timeout_secs: u64,
) -> CheckResult {
    let mut cmd = crate::core::platform::shell(&command);
    cmd.current_dir(repo_path)
        // Killed if this future is dropped (e.g. on timeout) instead of leaking.
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Own process group so a timeout can reap the whole tree (cross-platform helper).
    crate::core::platform::detach_process_group(&mut cmd);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                name,
                command,
                ok: false,
                code: -1,
                stdout: String::new(),
                stderr: e.to_string(),
            }
        }
    };
    // pgid == child pid（detach 后）。纳入 CPU 预算（Linux 且启用时），让门的 rustc/tsc
    // 也受总预算约束；超时再据此整组回收。
    let pgid = child.id();
    if let Some(p) = pgid {
        crate::core::cpubudget::attach(p);
    }

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await;

    match output {
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            CheckResult {
                name,
                command,
                ok: output.status.success(),
                code,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            }
        }
        Ok(Err(e)) => CheckResult {
            name,
            command,
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: e.to_string(),
        },
        Err(_) => {
            // 超时：SIGKILL 整个进程组（sh + cargo/tsc 子进程），不留孤儿。
            if let Some(p) = pgid {
                crate::core::reaper::kill_group(p);
            }
            CheckResult {
                name,
                command,
                ok: false,
                code: -1,
                stdout: String::new(),
                stderr: format!("timeout after {}s", timeout_secs),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // run_check 已从 `.output()` 重构为 spawn + cgroup attach + 超时整组真杀；
    // 这几条覆盖成功取输出 / 失败码 / 超时三条路径，守住重构正确性。
    #[tokio::test]
    async fn run_check_captures_success_output() {
        let r = run_check(".", "echo".into(), "echo af-ok-123".into(), 10).await;
        assert!(r.ok, "echo should succeed: {}", r.stderr);
        assert_eq!(r.code, 0);
        assert!(r.stdout.contains("af-ok-123"), "stdout={}", r.stdout);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_check_reports_failure_code() {
        let r = run_check(".", "exit3".into(), "exit 3".into(), 10).await;
        assert!(!r.ok);
        assert_eq!(r.code, 3);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_check_times_out_and_reaps() {
        let r = run_check(".", "sleep".into(), "sleep 30".into(), 1).await;
        assert!(!r.ok);
        assert!(r.stderr.contains("timeout"), "stderr={}", r.stderr);
    }
}
