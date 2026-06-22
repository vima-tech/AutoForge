//! 把「编码 Agent 技能（Skill）」注入到 spawn 出去的 CLI——让编码 agent 在 worktree 内
//! 消费可复用的指令包。这是 [`mcp_inject`](super::mcp_inject)（注入活的 MCP server）之外
//! 的第三条注入路：skill 是**静态指令包**，不需要活进程。
//!
//! 三家 CLI 机制不同（本模块按 kind 各自生成）：
//! - **claude**（一等公民）：写 `<worktree>/.claude/skills/<name>/SKILL.md`，claude 原生
//!   渐进披露（name+description 常驻上下文，body 按需加载）。`--print` 非交互下 `Skill`
//!   工具需经 `--allowedTools Skill` 显式放行（镜像 mcp 的 `--allowedTools mcp__<name>`）。
//! - **codex / opencode**：无原生 skill 机制 → 降级为把 skill 折叠进 prompt（当普通指令）。
//!   这比 mcp 的 opencode-noop 略强，因为 skill 本质就是指令、不需要活 server。
//!
//! 纯 Rust 零 Tauri。无 skill 时所有产出为空，CLI 命令与 prompt 与改造前**字节一致**（零回归）。
//!
//! **防提交污染（关键）**：claude 注入的 `.claude/skills/<name>` 写在 worktree 内，而 worktree
//! 在 run 之后会被 `git commit` 上 CR 分支。故：① 每个注入目录用 [`SkillDir`] Drop 守卫，在
//! `run()` 作用域结束（早于 execution 的 commit）即删除；② 同时把 `.claude/skills/` 写进
//! worktree 的 `.git/info/exclude`，即便清理 race 也不会被 commit（exclude 只影响**未跟踪**
//! 文件，故不会动到仓库自带、已跟踪的 `.claude/skills`）。

use std::path::{Path, PathBuf};

/// 解密后、可直接写进子进程配置的技能规格（与 DB 行解耦）。
#[derive(Debug, Clone)]
pub struct SkillInject {
    /// 清洗后的技能名（作 SKILL.md frontmatter name 与目录名）。
    pub name: String,
    /// 一行能力描述（渐进披露：决定 claude 何时加载 body）。
    pub description: String,
    /// SKILL.md 正文（Markdown）。
    pub body: String,
}

impl SkillInject {
    /// 从 DB 行构造（清洗名字、限长 body）。
    pub fn from_row(r: &crate::models::code_agent_skill::CodeAgentSkillRow) -> Self {
        Self {
            name: sanitize(&r.name),
            description: r.description.replace(['\n', '\r'], " ").trim().to_string(),
            body: r.body.clone(),
        }
    }
}

/// 名字清洗为 `[A-Za-z0-9_-]`，与 mcp_inject / tools::mcp 的前缀规则一致。
pub fn sanitize(s: &str) -> String {
    let out: String = s
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "skill".to_string() } else { out }
}

/// 单条 body 折叠进 prompt 时的限长，避免撑爆命令行/上下文。
const FOLD_BODY_CHARS: usize = 4000;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}\n…（已截断）")
    }
}

/// worktree 内某个被我们注入的 skill 目录的清理守卫：Drop 时递归删除该目录，
/// 并按需删除我们创建的空 `skills` / `.claude` 父目录（绝不碰仓库自带的）。
pub struct SkillDir {
    dir: PathBuf,
    /// `.claude/skills` 是否由我们创建（之前不存在）→ 清空后删除。
    created_skills_root: Option<PathBuf>,
    /// `.claude` 是否由我们创建（之前不存在）→ 清空后删除。
    created_claude_root: Option<PathBuf>,
}

impl Drop for SkillDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        // 仅当父目录由我们创建且已空时删除（remove_dir 只删空目录，天然安全）。
        if let Some(p) = &self.created_skills_root {
            let _ = std::fs::remove_dir(p);
        }
        if let Some(p) = &self.created_claude_root {
            let _ = std::fs::remove_dir(p);
        }
    }
}

/// 一次注入产出：需放行的工具名、需保活到进程退出的目录守卫、折叠进 prompt 的文本。
#[derive(Default)]
pub struct SkillCliConfig {
    /// claude：需放行的工具名（确有注入时含 `Skill`）。由 cli.rs 与 mcp 放行项合并发出。
    pub allowed_tools: Vec<String>,
    /// claude：写入 worktree 的 skill 目录守卫（Drop 时清理，早于 commit）。
    pub temp_dirs: Vec<SkillDir>,
    /// codex/opencode：折叠进 prompt 的技能段（claude 为空）。
    pub prompt_appendix: String,
}

/// 按 kind 分发。无 skill 时返回空配置（零回归）。
pub fn build(kind: &str, worktree: &str, skills: &[SkillInject]) -> SkillCliConfig {
    if skills.is_empty() {
        return SkillCliConfig::default();
    }
    match kind {
        "codex" | "opencode" => fold_into_prompt(skills),
        // 默认 = claude（一等公民）。
        _ => for_claude(worktree, skills),
    }
}

/// claude：把每条技能写成 `<worktree>/.claude/skills/<name>/SKILL.md`，返回 `--allowedTools Skill`。
/// 仓库已自带同名 SKILL.md（已是原生技能）则跳过、不接管、不删除。
pub fn for_claude(worktree: &str, skills: &[SkillInject]) -> SkillCliConfig {
    let mut cfg = SkillCliConfig::default();
    let claude_root = Path::new(worktree).join(".claude");
    let skills_root = claude_root.join("skills");
    let claude_existed = claude_root.exists();
    let skills_existed = skills_root.exists();

    let mut written: Vec<PathBuf> = Vec::new();
    for s in skills {
        let dir = skills_root.join(&s.name);
        let md = dir.join("SKILL.md");
        // 仓库已有该技能（手写在 repo 里）→ 让 repo 赢、原样保留，不接管也不删。
        if md.exists() {
            continue;
        }
        if std::fs::create_dir_all(&dir).is_err() {
            continue; // best-effort：写盘失败不阻断执行。
        }
        let content = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            s.name, s.description, s.body
        );
        if std::fs::write(&md, content).is_ok() {
            written.push(dir);
        }
    }

    if written.is_empty() {
        return SkillCliConfig::default(); // 全部跳过（仓库已自带）→ 命令不变。
    }

    // 父目录清理挂到**最后**一个守卫：Vec 按 index 0→n 顺序 drop，故最后元素 drop 时
    // 所有 per-skill 目录已删，此时 `remove_dir` 空的父目录才会成功（只有我们创建的才删）。
    let last = written.len() - 1;
    cfg.temp_dirs = written
        .into_iter()
        .enumerate()
        .map(|(i, dir)| SkillDir {
            dir,
            created_skills_root: (i == last && !skills_existed).then(|| skills_root.clone()),
            created_claude_root: (i == last && !claude_existed).then(|| claude_root.clone()),
        })
        .collect();
    // 防污染兜底：未跟踪的注入文件不进 git status / add -A。
    exclude_in_worktree(worktree, ".claude/skills/");
    // `--print` 非交互下需显式放行 Skill 工具（镜像 mcp 的 allowedTools 放行）；
    // 令牌交给 cli.rs 与 mcp 放行项合并成一次 `--allowedTools`。
    cfg.allowed_tools.push("Skill".to_string());
    cfg
}

/// codex/opencode：把技能折叠成一段可拼进 prompt 的指令文本（无原生 skill 机制时的降级）。
pub fn fold_into_prompt(skills: &[SkillInject]) -> SkillCliConfig {
    let mut s = String::from("\n## 可用技能（按需遵循）\n");
    for sk in skills {
        s.push_str(&format!(
            "\n### 技能：{} — {}\n{}\n",
            sk.name,
            sk.description,
            clip(&sk.body, FOLD_BODY_CHARS)
        ));
    }
    SkillCliConfig { prompt_appendix: s, ..Default::default() }
}

/// 把一行 pattern 追加进 worktree 的 `.git/info/exclude`（去重、幂等、best-effort）。
fn exclude_in_worktree(worktree: &str, pattern: &str) {
    // worktree 的 git 目录可能是文件（`.git` 指向主仓 worktrees/<id>）；用 rev-parse 取真实
    // info 目录最稳，但为零依赖、不 spawn，这里直接走常见布局：<worktree>/.git/info/exclude。
    // worktree 链接式 .git 时该路径不存在 → 写失败被忽略，仅丢失兜底（Drop 仍清理，不影响正确性）。
    let info = Path::new(worktree).join(".git").join("info");
    let exclude = info.join("exclude");
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == pattern) {
        return;
    }
    let _ = std::fs::create_dir_all(&info);
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(pattern);
    next.push('\n');
    let _ = std::fs::write(&exclude, next);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, body: &str) -> SkillInject {
        SkillInject {
            name: sanitize(name),
            description: "测试技能".to_string(),
            body: body.to_string(),
        }
    }

    fn tmp_worktree() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "af-skill-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize("My Skill!"), "My-Skill");
        assert_eq!(sanitize("  ../etc  "), "etc");
        assert_eq!(sanitize("***"), "skill");
        assert_eq!(sanitize("ok_name-1"), "ok_name-1");
    }

    #[test]
    fn empty_skills_is_zero_regression() {
        let cfg = build("claude", "/tmp", &[]);
        assert!(cfg.allowed_tools.is_empty());
        assert!(cfg.temp_dirs.is_empty());
        assert!(cfg.prompt_appendix.is_empty());
    }

    #[test]
    fn claude_writes_skill_and_allowlist_then_cleans_up() {
        let wt = tmp_worktree();
        let wt_s = wt.to_string_lossy().to_string();
        let skill_md = wt.join(".claude").join("skills").join("my-skill").join("SKILL.md");
        {
            let cfg = for_claude(&wt_s, &[skill("my skill", "do the thing")]);
            assert!(cfg.allowed_tools.iter().any(|a| a == "Skill"));
            assert_eq!(cfg.temp_dirs.len(), 1);
            // 文件真的写了，含 frontmatter。
            let content = std::fs::read_to_string(&skill_md).unwrap();
            assert!(content.contains("name: my-skill"));
            assert!(content.contains("do the thing"));
            // 兜底 exclude 已写。
            let excl = std::fs::read_to_string(wt.join(".git").join("info").join("exclude")).unwrap();
            assert!(excl.contains(".claude/skills/"));
        }
        // 守卫 drop 后目录被清理（不会被 commit）；我们创建的 .claude 父目录也清掉。
        assert!(!skill_md.exists());
        assert!(!wt.join(".claude").exists());
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[test]
    fn claude_skips_repo_authored_skill() {
        let wt = tmp_worktree();
        let wt_s = wt.to_string_lossy().to_string();
        // 仓库已自带同名技能。
        let repo_dir = wt.join(".claude").join("skills").join("repo-skill");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::write(repo_dir.join("SKILL.md"), "REPO ORIGINAL").unwrap();
        {
            let cfg = for_claude(&wt_s, &[skill("repo-skill", "INJECTED")]);
            // 全部跳过 → 空配置，命令不变。
            assert!(cfg.allowed_tools.is_empty());
            assert!(cfg.temp_dirs.is_empty());
        }
        // 仓库原文件未被接管/覆盖/删除。
        assert_eq!(
            std::fs::read_to_string(repo_dir.join("SKILL.md")).unwrap(),
            "REPO ORIGINAL"
        );
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[test]
    fn codex_folds_into_prompt() {
        let cfg = build("codex", "/tmp", &[skill("audit", "check races")]);
        assert!(cfg.allowed_tools.is_empty());
        assert!(cfg.temp_dirs.is_empty());
        assert!(cfg.prompt_appendix.contains("可用技能"));
        assert!(cfg.prompt_appendix.contains("audit"));
        assert!(cfg.prompt_appendix.contains("check races"));
    }

    #[test]
    fn fold_clips_long_body() {
        let long = "x".repeat(FOLD_BODY_CHARS + 500);
        let cfg = fold_into_prompt(&[skill("big", &long)]);
        assert!(cfg.prompt_appendix.contains("已截断"));
    }
}
