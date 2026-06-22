use anyhow::Result;
use async_trait::async_trait;

pub mod cli;
pub mod mcp_inject;
pub mod skill_inject;
pub use cli::{CliCodeAgent, CliProfile};
pub use mcp_inject::McpInject;
pub use skill_inject::SkillInject;

/// 加载「适用于编码 Agent」的 MCP server（for_code_agent=1 且 enabled=1），解密后供 CLI 注入（pull）。
/// 查询失败 / 无配置 → 空 Vec（编码 agent 不接任何实时 MCP，行为与改造前一致）。
pub async fn load_code_agent_mcp(db: &crate::db::Db) -> Vec<McpInject> {
    sqlx::query_as::<_, crate::models::mcp_server::McpServer>(
        "SELECT * FROM mcp_servers WHERE for_code_agent=1 AND enabled=1 ORDER BY created_at",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .iter()
    .map(McpInject::from_server)
    .collect()
}

/// 加载「适用于编码 Agent」的技能（skill），供 CLI 注入（claude 写 SKILL.md / 其余折叠进 prompt）。
/// 两路来源取并集，按 name 去重，**项目级文件覆盖同名全局库条目**：
///   ① 全局库 `code_agent_skills`（enabled，且 project_id 为 NULL 或 = 本项目）；
///   ② 项目级 `<repo>/.autoforge/skills/<name>/SKILL.md`（与 .autoforge/specs 同构，仓内手写）。
/// 查询失败 / 无配置 → 空 Vec（编码 agent 不接任何技能，行为与改造前一致）。
pub async fn load_code_agent_skills(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
) -> Vec<SkillInject> {
    use crate::models::code_agent_skill::CodeAgentSkillRow;
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, SkillInject> = BTreeMap::new();
    // ① 全局库（含本项目专属）。
    let rows = sqlx::query_as::<_, CodeAgentSkillRow>(
        "SELECT * FROM code_agent_skills
         WHERE enabled=1 AND (project_id IS NULL OR project_id=?)
         ORDER BY created_at",
    )
    .bind(&project.id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    for r in &rows {
        let s = SkillInject::from_row(r);
        by_name.insert(s.name.clone(), s);
    }
    // ② 项目级文件覆盖（仓内手写优先）。
    for s in read_project_skills(&project.repo_path) {
        by_name.insert(s.name.clone(), s);
    }
    by_name.into_values().collect()
}

/// 读 `<repo>/.autoforge/skills/<name>/SKILL.md`：解析 frontmatter（name/description）+ 正文。
/// 与 build_prompt 里 read_project_specs 同构（同步、best-effort、每文件限长）。无目录 → 空。
fn read_project_skills(repo_path: &str) -> Vec<SkillInject> {
    let dir = std::path::Path::new(repo_path)
        .join(".autoforge")
        .join("skills");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for entry in rd.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let md = entry.path().join("SKILL.md");
        let Ok(content) = std::fs::read_to_string(&md) else {
            continue;
        };
        let dir_name = entry.file_name().to_string_lossy().to_string();
        out.push(parse_skill_md(&dir_name, &content));
    }
    out
}

/// 解析 SKILL.md：`---\nname: ..\ndescription: ..\n---\n<body>`。缺 frontmatter 时
/// name 回退目录名、description 回退首行、body 取全文（容错，绝不 panic）。
fn parse_skill_md(dir_name: &str, content: &str) -> SkillInject {
    let mut name = skill_inject::sanitize(dir_name);
    let mut description = String::new();
    let trimmed = content.trim_start();
    let body = if let Some(rest) = trimmed.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            for line in front.lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    let v = v.trim();
                    if !v.is_empty() {
                        name = skill_inject::sanitize(v);
                    }
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                }
            }
            // 跳过 frontmatter 结束的 `\n---` 及其后换行。
            rest[end + 4..].trim_start_matches('\n').to_string()
        } else {
            content.to_string()
        }
    } else {
        content.to_string()
    };
    if description.is_empty() {
        description = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    }
    SkillInject { name, description, body }
}

/// 单条日志正文（stdout / stderr）落库上限——超出只保留**尾部**这么多字符并打标记。
/// 失败/超时的关键线索（最后报错、卡死前最后输出）都在尾部，故截头保尾。
const LOG_KEEP_CHARS: usize = 512 * 1024;
/// 运行日志保留窗口（天）：滚动清理，保证持久可查又不无限膨胀。
const LOG_RETENTION_DAYS: i64 = 14;

/// 截头保尾到 `LOG_KEEP_CHARS`，返回 (落库文本, 原始字符数, 是否截断)。
fn clip_tail(s: &str) -> (String, i64, bool) {
    let total = s.chars().count();
    if total <= LOG_KEEP_CHARS {
        return (s.to_string(), total as i64, false);
    }
    let tail: String = s
        .chars()
        .skip(total - LOG_KEEP_CHARS)
        .collect();
    let dropped = total - LOG_KEEP_CHARS;
    (
        format!("…（已省略开头 {dropped} 个字符）\n{tail}"),
        total as i64,
        true,
    )
}

/// 一次代码 Agent 运行的留档输入。纯数据，无 Tauri 类型。
pub struct RunLogInput<'a> {
    pub change_request_id: &'a str,
    pub worktree_session_id: Option<&'a str>,
    /// execution（代码实现）/ conflict_resolve（AI 解合并冲突）。
    pub phase: &'a str,
    pub kind: &'a str,
    pub model: Option<&'a str>,
    pub exit_code: i32,
    pub stdout: &'a str,
    pub stderr: &'a str,
    pub duration_ms: i64,
}

/// 把一次代码 Agent 执行的完整 stdout/stderr 落库（迁移 0064 表），并滚动清理过期日志。
/// 失败只记录不阻断——留档不应影响主流程。纯 Rust（仅依赖 Db），可在任何入口复用。
pub async fn log_run(db: &crate::db::Db, input: RunLogInput<'_>) {
    let (stdout, stdout_bytes, t1) = clip_tail(input.stdout);
    let (stderr, stderr_bytes, t2) = clip_tail(input.stderr);
    let id = uuid::Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO code_agent_run_logs
         (id, change_request_id, worktree_session_id, phase, kind, model, exit_code,
          duration_ms, stdout, stderr, stdout_bytes, stderr_bytes, truncated)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(input.change_request_id)
    .bind(input.worktree_session_id)
    .bind(input.phase)
    .bind(input.kind)
    .bind(input.model)
    .bind(input.exit_code as i64)
    .bind(input.duration_ms)
    .bind(&stdout)
    .bind(&stderr)
    .bind(stdout_bytes)
    .bind(stderr_bytes)
    .bind(i64::from(t1 || t2))
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("code agent run log insert failed: {e}");
        return;
    }
    // 滚动清理：超过保留窗口的日志删除（带索引，开销极低）。
    let _ = sqlx::query("DELETE FROM code_agent_run_logs WHERE created_at < datetime('now', ?)")
        .bind(format!("-{LOG_RETENTION_DAYS} days"))
        .execute(db)
        .await;
}

/// 一次 agent 运行的时限。`wall_secs` 是硬上限（墙钟）；`idle_secs` 是空闲超时——
/// 连续这么久没有任何输出即判定卡死（0 = 关闭空闲超时）。两者任一触发都会对整个
/// 进程组发 SIGKILL，回收 agent 及其 ripgrep/构建子进程，避免孤儿持续烧 CPU。
#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub wall_secs: u64,
    pub idle_secs: u64,
}

/// 一段运行期实时日志增量：`stream` 为 "stdout"/"stderr"，`text` 为该段可读文本。
#[derive(Debug, Clone)]
pub struct LogChunk {
    pub stream: &'static str,
    pub text: String,
}

/// 运行期实时日志的 sink。cli.rs 边跑边往里 `send`（纯 Rust，零 Tauri 依赖）；
/// 持有 AppHandle 的任务层（execution/merge）从中收取并经
/// `event::emit(AppEvent::CodeAgentLog)` 推前端，实现"实时滚动日志"。
pub type LogSink = tokio::sync::mpsc::UnboundedSender<LogChunk>;

/// 可插拔代码实现 agent 的统一抽象。纯 Rust，零 Tauri 类型——业务层只依赖此 trait，
/// 不感知底层是哪个 CLI（claude / codex / opencode），未来可换非 CLI 实现。
#[async_trait]
pub trait CodeAgent: Send + Sync {
    /// 在 worktree 内执行实现任务，返回 (exit_code, stdout, stderr)。超时（墙钟或空闲）
    /// 会真正杀掉子进程组并以退出码 124 返回（保留已捕获输出），绝不留下孤儿进程。
    /// `mcp` 是「适用于编码 Agent」的 MCP server（pull）：注入 CLI 让 agent 实时调用；
    /// 空切片 = 不接任何实时 MCP。`skills` 是「编码 Agent 技能」：claude 写 worktree
    /// `.claude/skills` 走原生渐进披露、其余折叠进 prompt；空切片 = 不注入任何技能（行为不变）。
    /// `log` 为可选实时日志 sink：每收到一段 stdout/stderr 即往里 send，供上层推前端做"实时
    /// 滚动"；None = 不需要实时（行为不变）。
    async fn run(
        &self,
        worktree: &str,
        prompt: &str,
        limits: RunLimits,
        mcp: &[McpInject],
        skills: &[SkillInject],
        log: Option<&LogSink>,
    ) -> Result<(i32, String, String)>;
    /// 该 agent 是否已安装并（在可探测时）登录。
    async fn check_auth(&self) -> bool;
    /// kind 标识（claude / codex / opencode）。
    fn kind(&self) -> &str;
}

/// 一次代码实现的风险档位，决定选哪个模型（快 / 强）。由分析阶段的结构化信号
/// （blast_radius + 影响文件数 + 复杂度）在执行**前**推断——grader 在执行后才跑，
/// 来不及用来选模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRisk {
    /// 隔离的小改动：用快模型省时省钱。
    Low,
    /// 跨模块 / 系统级 / 复杂改动：用强模型保质量。
    High,
}

/// 从分析 spec 推断风险档位。无 spec 时保守按 High（用强模型），绝不为省钱牺牲质量。
pub fn risk_from_spec(
    spec: Option<&crate::agents::analysis::IssueAnalysisSpec>,
) -> CodeRisk {
    let Some(spec) = spec else { return CodeRisk::High };

    // 影响半径：cross_module / systemic 一律 High。
    match spec.scope.blast_radius.as_str() {
        "cross_module" | "systemic" => return CodeRisk::High,
        _ => {}
    }
    // 复杂度：high / complex 视为 High。
    if let Some(est) = spec.estimate.as_ref() {
        let c = est.complexity.to_ascii_lowercase();
        if c.contains("high") || c.contains("complex") || c.contains("高") {
            return CodeRisk::High;
        }
    }
    // 影响面：触及超过 3 个文件视为 High。
    let touched = spec.scope.affected_files.len();
    if touched > 3 {
        return CodeRisk::High;
    }
    // 其余（isolated / module、少量文件、低复杂度）→ Low，用快模型。
    CodeRisk::Low
}

/// 按「项目覆盖 → 全局默认 → 硬兜底 claude」选出本次该用的 code agent 行。
/// 表不存在 / 查询失败 / 无启用项时返回 None（调用方回落硬兜底 claude）。
async fn resolve_row(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
) -> Option<crate::models::code_agent::CodeAgentRow> {
    use crate::models::code_agent::CodeAgentRow;

    // 1) 项目级覆盖（且该 agent 启用）。
    let row: Option<CodeAgentRow> = if let Some(id) =
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
    if row.is_some() {
        return row;
    }

    // 2) 全局默认（启用）。
    sqlx::query_as::<_, CodeAgentRow>(
        "SELECT * FROM code_agents WHERE is_default=1 AND enabled=1 LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// 解析 code agent，模型用该行配置的 `model`（不分级）。AI 解冲突等场景用这个。
pub async fn resolve(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
) -> Box<dyn CodeAgent> {
    match resolve_row(db, project).await {
        Some(r) => Box::new(CliCodeAgent::new(CliProfile {
            kind: r.kind,
            program: r.program,
            model: r.model,
            extra_args: parse_extra_args(&r.extra_args_json),
        })),
        None => Box::new(CliCodeAgent::claude()),
    }
}

/// 解析 code agent，并按风险档位挑模型：Low → fast_model、High → strong_model；
/// 选中的那档为空时回落到 `model`（再空则交给底层 CLI 默认）。三家共用此机制。
/// 返回 (agent, 实际选用的模型字符串/None)，供进度提示展示。
pub async fn resolve_for_risk(
    db: &crate::db::Db,
    project: &crate::models::project::Project,
    risk: CodeRisk,
) -> (Box<dyn CodeAgent>, Option<String>) {
    match resolve_row(db, project).await {
        Some(r) => {
            let tiered = match risk {
                CodeRisk::Low => r.fast_model.clone(),
                CodeRisk::High => r.strong_model.clone(),
            };
            // 选中档为空 → 回落 model。统一在此 trim 掉空串，避免传空 --model。
            let model = tiered
                .or_else(|| r.model.clone())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let agent = Box::new(CliCodeAgent::new(CliProfile {
                kind: r.kind,
                program: r.program,
                model: model.clone(),
                extra_args: parse_extra_args(&r.extra_args_json),
            }));
            (agent, model)
        }
        None => (Box::new(CliCodeAgent::claude()), None),
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
    codegraph_ctx: Option<&str>,
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

    // codegraph 预查的精确定位（file:line + 签名 + 调用者），紧跟 spec 的影响范围之后，
    // 让 agent 拿到可直接打开的位置，省去全仓探索。空串时不输出任何标题。
    if let Some(cg) = codegraph_ctx {
        if !cg.trim().is_empty() {
            prompt.push_str(cg);
        }
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

    #[test]
    fn parse_skill_md_reads_frontmatter() {
        let md = "---\nname: race-audit\ndescription: 审查并发竞态\n---\n\n# 正文\n逐项检查锁。";
        let s = parse_skill_md("dir-name", md);
        assert_eq!(s.name, "race-audit");
        assert_eq!(s.description, "审查并发竞态");
        assert!(s.body.contains("逐项检查锁"));
        assert!(!s.body.starts_with("---")); // frontmatter 已剥离
    }

    #[test]
    fn parse_skill_md_without_frontmatter_falls_back() {
        let s = parse_skill_md("my dir", "第一行说明\n更多内容");
        assert_eq!(s.name, "my-dir"); // 回退目录名（已清洗）
        assert_eq!(s.description, "第一行说明"); // 回退首行
        assert!(s.body.contains("更多内容"));
    }

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
    fn risk_from_spec_classifies_low_vs_high() {
        use crate::agents::analysis::{AffectedFile, Estimate, IssueAnalysisSpec};
        // 无 spec → 保守 High。
        assert_eq!(risk_from_spec(None), CodeRisk::High);

        // 隔离 + 少量文件 + 低复杂度 → Low。
        let mut spec = IssueAnalysisSpec::default();
        spec.scope.blast_radius = "isolated".into();
        spec.scope.affected_files = vec![AffectedFile::default()];
        assert_eq!(risk_from_spec(Some(&spec)), CodeRisk::Low);

        // 影响半径 systemic → High。
        let mut s2 = spec.clone();
        s2.scope.blast_radius = "systemic".into();
        assert_eq!(risk_from_spec(Some(&s2)), CodeRisk::High);

        // 文件数 > 3 → High。
        let mut s3 = spec.clone();
        s3.scope.affected_files = vec![AffectedFile::default(); 4];
        assert_eq!(risk_from_spec(Some(&s3)), CodeRisk::High);

        // 复杂度 high → High。
        let mut s4 = spec.clone();
        s4.estimate = Some(Estimate { complexity: "high".into(), ..Default::default() });
        assert_eq!(risk_from_spec(Some(&s4)), CodeRisk::High);
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
