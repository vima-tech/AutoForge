-- 编码 Agent 技能（Skill）：注入到 worktree 让编码 agent 消费的可复用指令包。
-- claude 一等公民——写 <worktree>/.claude/skills/<name>/SKILL.md 走原生渐进披露；
-- codex/opencode 无原生 skill 机制，降级为折叠进 prompt。
-- 与 .autoforge/skills/ 文件级来源取并集（项目级文件覆盖同名全局库条目）。
CREATE TABLE IF NOT EXISTS code_agent_skills (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,           -- 技能名（渐进披露键 + SKILL.md frontmatter name + 目录名）
  description TEXT NOT NULL,           -- 一行能力描述：决定 claude 何时按需加载 body
  body        TEXT NOT NULL,           -- SKILL.md 正文（Markdown）
  project_id  TEXT,                    -- NULL=全局适用；否则仅该项目注入
  enabled     INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_code_agent_skills_scope
  ON code_agent_skills (enabled, project_id);
