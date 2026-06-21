use anyhow::Result;
use async_trait::async_trait;

pub mod cli;
pub use cli::{CliCodeAgent, CliProfile};

/// 一次 agent 运行的时限。`wall_secs` 是硬上限（墙钟）；`idle_secs` 是空闲超时——
/// 连续这么久没有任何输出即判定卡死（0 = 关闭空闲超时）。两者任一触发都会对整个
/// 进程组发 SIGKILL，回收 agent 及其 ripgrep/构建子进程，避免孤儿持续烧 CPU。
#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub wall_secs: u64,
    pub idle_secs: u64,
}

/// 可插拔代码实现 agent 的统一抽象。纯 Rust，零 Tauri 类型——业务层只依赖此 trait，
/// 不感知底层是哪个 CLI（claude / codex / opencode），未来可换非 CLI 实现。
#[async_trait]
pub trait CodeAgent: Send + Sync {
    /// 在 worktree 内执行实现任务，返回 (exit_code, stdout, stderr)。超时（墙钟或空闲）
    /// 会真正杀掉子进程组并返回 Err，绝不留下孤儿进程。
    async fn run(
        &self,
        worktree: &str,
        prompt: &str,
        limits: RunLimits,
    ) -> Result<(i32, String, String)>;
    /// 该 agent 是否已安装并（在可探测时）登录。
    async fn check_auth(&self) -> bool;
    /// kind 标识（claude / codex / opencode）。
    fn kind(&self) -> &str;
}

/// 按「项目覆盖 → 全局默认 → 硬兜底 claude」解析出本次该用的 code agent。
/// 表不存在 / 查询失败 / 无启用项时一律安全回落到 claude，绝不让解析失败阻断流水线。
pub async fn resolve(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
) -> Box<dyn CodeAgent> {
    use crate::models::code_agent::CodeAgentRow;

    // 1) 项目级覆盖（且该 agent 启用）。
    let mut row: Option<CodeAgentRow> = if let Some(id) =
        project.code_agent_id.as_deref().filter(|s| !s.is_empty())
    {
        sqlx::query_as::<_, CodeAgentRow>("SELECT * FROM code_agents WHERE id=? AND enabled=1")
            .bind(id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    // 2) 全局默认（启用）。
    if row.is_none() {
        row = sqlx::query_as::<_, CodeAgentRow>(
            "SELECT * FROM code_agents WHERE is_default=1 AND enabled=1 LIMIT 1",
        )
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    }

    match row {
        Some(r) => Box::new(CliCodeAgent::new(CliProfile {
            kind: r.kind,
            program: r.program,
            model: r.model,
            extra_args: parse_extra_args(&r.extra_args_json),
        })),
        // 3) 硬兜底。
        None => Box::new(CliCodeAgent::claude()),
    }
}

/// `extra_args_json` 存的是 JSON 字符串数组；解析失败按空处理（不阻断）。
pub fn parse_extra_args(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
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
0. 全自主执行：本任务在无人值守的流水线中运行，无法向用户提问或等待确认。遇到方案取舍、技术选型、命名等不确定点，**直接采用你判断下的最佳/推荐方案并落地实现**，不要停下来征询意见、不要只给建议而不动手；把所选方案与理由记到下方「改动摘要」即可。
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

/// Extract the report section starting at "## 改动摘要"
pub fn extract_report(output: &str) -> &str {
    if let Some(pos) = output.find("## 改动摘要") {
        &output[pos..]
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj(code_agent_id: Option<&str>) -> crate::models::project::Project {
        crate::models::project::Project {
            id: "p1".into(),
            name: "P".into(),
            slug: "p".into(),
            description: String::new(),
            repo_path: String::new(),
            branch_dev: "dev".into(),
            branch_main: "main".into(),
            status: "active".into(),
            config_yaml: None,
            is_default: false,
            archived_at: None,
            code_agent_id: code_agent_id.map(|s| s.to_string()),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    async fn mem_db() -> crate::db::Db {
        // 单连接池跑迁移，规避 sqlx 在多连接 SQLite 上的迁移竞态（仅测试用）。
        use sqlx::sqlite::SqlitePoolOptions;
        let dir = std::env::temp_dir().join(format!(
            "af-codeagent-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.join("t.db").display());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[test]
    fn parse_extra_args_tolerates_garbage() {
        assert!(parse_extra_args("[]").is_empty());
        assert_eq!(parse_extra_args(r#"["-a","b c"]"#), vec!["-a".to_string(), "b c".to_string()]);
        assert!(parse_extra_args("not json").is_empty());
    }

    #[test]
    fn extract_report_falls_back_to_full_output() {
        // marker 缺失（codex/opencode 可能不输出标题）→ 返回全文，不丢内容。
        assert_eq!(extract_report("done, edited foo.rs"), "done, edited foo.rs");
        assert_eq!(extract_report("noise\n## 改动摘要\nx"), "## 改动摘要\nx");
    }

    #[tokio::test]
    async fn resolve_priority_project_then_default_then_fallback() {
        let db = mem_db().await;
        // 种子默认 = claude。
        assert_eq!(resolve(&db, &proj(None)).await.kind(), "claude");
        // 项目级覆盖生效。
        assert_eq!(resolve(&db, &proj(Some("codex"))).await.kind(), "codex");
        // 覆盖项被禁用 → 回落全局默认。
        sqlx::query("UPDATE code_agents SET enabled=0 WHERE id='codex'")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(resolve(&db, &proj(Some("codex"))).await.kind(), "claude");
        // 未知 id → 回落默认。
        assert_eq!(resolve(&db, &proj(Some("ghost"))).await.kind(), "claude");
        // 换默认为 opencode 后，无覆盖的项目取 opencode。
        sqlx::query("UPDATE code_agents SET is_default=0 WHERE is_default=1")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE code_agents SET is_default=1 WHERE id='opencode'")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(resolve(&db, &proj(None)).await.kind(), "opencode");
    }
}
