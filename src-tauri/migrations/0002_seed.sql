INSERT OR IGNORE INTO llm_configs (id, name, provider, model, endpoint, temperature) VALUES
  ('llm-opus4',   'Claude Opus 4',   'Anthropic', 'claude-opus-4-20250514',   'https://api.anthropic.com', 0.3),
  ('llm-sonnet4', 'Claude Sonnet 4', 'Anthropic', 'claude-sonnet-4-20250514', 'https://api.anthropic.com', 0.2),
  ('llm-local',   '本地消毒模型',    'Ollama',    'qwen2.5:7b-instruct',      'http://localhost:11434',    0.0);

INSERT OR IGNORE INTO agents (id, name, name_en, role, color, initial, llm_id, system_prompt, forge_role) VALUES
  ('agent-analyst',   '需求分析师', 'Analysis Agent',  '需求真实性 · 可行性 · 优先级', '#8b7ad8', '析',  'llm-opus4',   '你是 AutoForge 的需求分析 Agent。评估需求真实性、可行性、优先级，输出结构化分析报告，检测重复需求。', 'analysis'),
  ('agent-coder',     'Claude Code', 'Execution Agent', '代码实现 · worktree · 初步测试', '#e8772e', 'CC', 'llm-sonnet4', '你是执行 Agent，在 worktree 内实现已审核的需求，遵守编码规范与权限边界，生成实现报告。', NULL),
  ('agent-tester',    '测试工程师', 'Test Agent',      '被动响应 · 主动巡检 · 缺陷报告', '#4f9d6b', '测', 'llm-sonnet4', '你是测试 Agent，验证功能正确性、执行回归与主动巡检，生成缺陷报告并自动入队。', 'test'),
  ('agent-architect', '架构顾问',   'Architect',       '技术选型 · 模块设计 · 重构建议', '#4f8ed1', '架', 'llm-opus4',   '你是架构顾问，对技术选型、模块边界和重构方案给出建议。', NULL),
  ('agent-security',  '安全审计',   'Security',        'Prompt 注入 · 权限边界 · 脱敏',  '#db5a40', '安', 'llm-sonnet4', '你是安全审计 Agent，识别 Prompt 注入、权限越界与数据脱敏风险。', NULL);
