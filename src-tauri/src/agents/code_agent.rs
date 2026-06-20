use anyhow::Result;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub fn build_prompt(
    title: &str,
    desc: &str,
    analysis_summary: &str,
    spec: Option<&crate::agents::analysis::IssueAnalysisSpec>,
    admin_suggestions: Option<&str>,
    iteration: u32,
    repo_path: &str,
    project_config: Option<&str>,
) -> String {
    let mut prompt = format!(
        r#"# 需求实现任务

## 需求标题
{}

## 需求描述
{}

## 分析摘要
{}
"#,
        title, desc, analysis_summary
    );

    // 结构化分析规格（issue_analysis.schema.json v1.0）—— 给 Claude Code 的精准工单
    if let Some(spec) = spec {
        prompt.push_str(&render_spec_brief(spec));
    }

    if let Some(s) = admin_suggestions {
        if !s.is_empty() {
            prompt.push_str(&format!("\n## 管理员建议\n{}\n", s));
        }
    }

    if iteration > 1 {
        prompt.push_str(&format!(
            "\n## 注意\n这是第 {} 次迭代，请参考之前的实现继续改进。\n",
            iteration
        ));
    }

    let specs = read_project_specs(repo_path);
    if !specs.trim().is_empty() {
        prompt.push_str(&format!("\n## 项目规范（.autoforge/specs）\n{}\n", specs));
    }
    if let Some(context) = read_project_file(repo_path, "CLAUDE.md") {
        prompt.push_str(&format!("\n## 目标项目 CLAUDE.md\n{}\n", context));
    }
    let discovered_config = read_project_file(repo_path, "autoforge.yaml");
    if let Some(config) = project_config.or(discovered_config.as_deref()) {
        prompt.push_str(&format!("\n## autoforge.yaml / 项目配置快照\n{}\n", config));
    }

    prompt.push_str(
        r#"
## 要求
1. 在当前 worktree 中实现上述需求
2. 编写必要的测试
3. 完成后输出实现报告，格式如下：

## 改动摘要
（简述做了什么）

## 修改文件列表
（列出修改的文件）

## 测试情况
（测试结果）

## 潜在风险
（可能的风险点）
"#,
    );

    prompt
}

/// Render the structured analysis spec into a Claude Code work-order section.
/// Only non-empty parts are emitted, so feature/bug specs stay concise.
fn render_spec_brief(spec: &crate::agents::analysis::IssueAnalysisSpec) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    let u = &spec.understanding;
    if !u.restated_issue.is_empty() || !u.problem_type.is_empty() {
        s.push_str("\n## 需求理解\n");
        if !u.problem_type.is_empty() {
            let _ = writeln!(s, "- 类型：{}", u.problem_type);
        }
        if !u.restated_issue.is_empty() {
            let _ = writeln!(s, "- 重述：{}", u.restated_issue);
        }
        if let Some(cur) = u.current_behavior.as_deref().filter(|v| !v.is_empty()) {
            let _ = writeln!(s, "- 当前行为：{}", cur);
        }
        if let Some(exp) = u.expected_behavior.as_deref().filter(|v| !v.is_empty()) {
            let _ = writeln!(s, "- 期望行为：{}", exp);
        }
        if !u.reproduction_steps.is_empty() {
            s.push_str("- 复现步骤：\n");
            for (i, step) in u.reproduction_steps.iter().enumerate() {
                let _ = writeln!(s, "  {}. {}", i + 1, step);
            }
        }
    }

    if let Some(rc) = &spec.root_cause {
        if !rc.hypothesis.is_empty() {
            s.push_str("\n## 根因分析\n");
            let _ = writeln!(s, "- 假设：{}", rc.hypothesis);
            for ev in &rc.evidence {
                let _ = writeln!(s, "- 证据：{}", ev);
            }
            for loc in &rc.suspected_locations {
                let sym = loc.symbol.as_deref().filter(|v| !v.is_empty()).map(|v| format!(" :: {}", v)).unwrap_or_default();
                let _ = writeln!(s, "- 可疑位置：{}{} — {}", loc.file, sym, loc.reason);
            }
        }
    }

    let sc = &spec.scope;
    if !sc.affected_files.is_empty() || !sc.entry_points.is_empty() {
        s.push_str("\n## 影响范围\n");
        if !sc.blast_radius.is_empty() {
            let _ = writeln!(s, "- 影响半径：{}", sc.blast_radius);
        }
        for f in &sc.affected_files {
            let _ = writeln!(s, "- [{}] {} — {}", f.change_type, f.path, f.reason);
        }
        if !sc.related_files.is_empty() {
            let _ = writeln!(s, "- 参考文件：{}", sc.related_files.join(", "));
        }
        if !sc.entry_points.is_empty() {
            let _ = writeln!(s, "- 入手点：{}", sc.entry_points.join("; "));
        }
        if !sc.out_of_scope.is_empty() {
            let _ = writeln!(s, "- 不在范围内：{}", sc.out_of_scope.join("; "));
        }
    }

    let plan = &spec.implementation_plan;
    if !plan.approach.is_empty() || !plan.steps.is_empty() {
        s.push_str("\n## 实现计划\n");
        if !plan.approach.is_empty() {
            let _ = writeln!(s, "{}\n", plan.approach);
        }
        let mut steps = plan.steps.clone();
        steps.sort_by_key(|st| st.order);
        for st in &steps {
            let files = if st.target_files.is_empty() { String::new() } else { format!("（{}）", st.target_files.join(", ")) };
            let _ = writeln!(s, "{}. {}{}", st.order, st.action, files);
            if let Some(d) = st.details.as_deref().filter(|v| !v.is_empty()) {
                let _ = writeln!(s, "   - {}", d);
            }
        }
        for dm in &plan.data_model_changes {
            if dm.kind != "none" && !dm.description.is_empty() {
                let _ = writeln!(s, "- 数据模型变更（{}）：{}", dm.kind, dm.description);
            }
        }
        if !plan.new_dependencies.is_empty() {
            let _ = writeln!(s, "- 新增依赖（需谨慎）：{}", plan.new_dependencies.join(", "));
        }
    }

    if !spec.acceptance_criteria.is_empty() {
        s.push_str("\n## 验收标准\n");
        for ac in &spec.acceptance_criteria {
            let _ = writeln!(s, "- {} {}", ac.id, ac.statement);
        }
    }

    let c = &spec.constraints;
    if !c.must.is_empty() || !c.must_not.is_empty() {
        s.push_str("\n## 约束\n");
        for m in &c.must {
            let _ = writeln!(s, "- 必须：{}", m);
        }
        for m in &c.must_not {
            let _ = writeln!(s, "- 禁止：{}", m);
        }
    }

    if !spec.risks.is_empty() {
        s.push_str("\n## 风险\n");
        for r in &spec.risks {
            let mit = r.mitigation.as_deref().filter(|v| !v.is_empty()).map(|v| format!("（缓解：{}）", v)).unwrap_or_default();
            let _ = writeln!(s, "- [{}] {}{}", r.severity, r.description, mit);
        }
    }

    let b = &spec.claude_code_brief;
    if !b.objective.is_empty() || !b.instructions.is_empty() {
        s.push_str("\n## 执行工单（务必遵循）\n");
        if !b.objective.is_empty() {
            let _ = writeln!(s, "目标：{}", b.objective);
        }
        if !b.instructions.is_empty() {
            s.push_str("步骤：\n");
            for (i, ins) in b.instructions.iter().enumerate() {
                let _ = writeln!(s, "  {}. {}", i + 1, ins);
            }
        }
        for d in &b.r#do {
            let _ = writeln!(s, "- ✅ {}", d);
        }
        for d in &b.dont {
            let _ = writeln!(s, "- ❌ {}", d);
        }
        if !b.files_to_touch.is_empty() {
            let _ = writeln!(s, "- 预计改动文件：{}", b.files_to_touch.join(", "));
        }
        if !b.definition_of_done.is_empty() {
            s.push_str("- 完成判定：\n");
            for d in &b.definition_of_done {
                let _ = writeln!(s, "  - {}", d);
            }
        }
    }

    if !spec.open_questions.is_empty() {
        s.push_str("\n## 待澄清（如阻塞实现，请在报告中说明）\n");
        for q in &spec.open_questions {
            let _ = writeln!(s, "- {}", q);
        }
    }

    s
}

/// Read all per-project specs from `<repo>/.autoforge/specs/*.md` — the single
/// source of truth for the target project's standards. Returns concatenated
/// markdown (each file under a `### <file>` header), capped per file. Empty when
/// the dir is absent/empty. Sync sibling of `analysis::read_autoforge_specs`.
fn read_project_specs(repo_path: &str) -> String {
    let dir = std::path::Path::new(repo_path).join(".autoforge").join("specs");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return String::new();
    };

    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".md"))
        .collect();
    names.sort();

    let mut parts: Vec<String> = Vec::new();
    for name in names {
        if let Ok(content) = std::fs::read_to_string(dir.join(&name)) {
            let trimmed: String = content.chars().take(6000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("### {}\n{}", name, trimmed));
            }
        }
    }
    parts.join("\n\n")
}

fn read_project_file(repo_path: &str, name: &str) -> Option<String> {
    std::fs::read_to_string(std::path::Path::new(repo_path).join(name)).ok()
}

/// Run claude code agent in a worktree
/// Returns (exit_code, stdout, stderr)
pub async fn run(
    worktree_path: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<(i32, String, String)> {
    // The prompt is fed via stdin rather than as a positional argument: claude's
    // `--disallowedTools` flag is variadic and would otherwise greedily consume
    // the prompt as a list of tool names, leaving the run with no actual prompt
    // ("Input must be provided ... when using --print").
    // Resolve `claude` via PATH so Windows finds the `claude.cmd` npm shim
    // (`Command::new` only auto-appends `.exe`); no-op on unix.
    let mut cmd = Command::new(crate::core::platform::program("claude"));
    cmd.arg("--print")
        .arg("--permission-mode")
        .arg("acceptEdits")
        .arg("--disallowedTools")
        .arg("Bash(git *)")
        .current_dir(worktree_path)
        // Defense-in-depth: `--disallowedTools "Bash(git *)"` only blocks the
        // direct `git ` form; the agent could still shell out via `sh -c`,
        // scripts, etc. Disabling all git transport protocols neutralizes any
        // remote git operation (push/fetch/clone) no matter how it is invoked,
        // while local commits/builds/tests in the worktree keep working.
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await?; // close stdin so claude stops waiting for input
    }

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("claude code agent timed out after {}s", timeout_secs))??;

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((code, stdout, stderr))
}

/// Extract the report section starting at "## 改动摘要"
pub fn extract_report(output: &str) -> &str {
    if let Some(pos) = output.find("## 改动摘要") {
        &output[pos..]
    } else {
        output
    }
}
