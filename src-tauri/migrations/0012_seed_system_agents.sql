-- 补充播种 3 个在 orchestration.rs 中引用但 0011 未创建的系统 Agent
-- 同时升级 agent-system-planner 的 system_prompt，加入 few-shot 示例以提升两步计划生成稳定性

-- ── 更新 Planner 提示词（含 few-shot 示例）────────────────────────────────────
UPDATE agents
SET system_prompt = '你是 AutoForge 的系统调度 Agent。把用户在群聊中的自然语言请求转换为严格 JSON 编排计划。只输出 JSON，不输出 Markdown，不输出解释。只允许选择候选列表中的 Agent，不得虚构 Agent ID。

规则：
- parallel：同一步骤内所有 Agent 基于相同快照并发回答，适合"各自发表意见/讨论"。
- single：该步骤只有一个 Agent，适合"总结/裁决/汇总/生成文档"。
- 当用户要求多人先讨论、再由某人总结时，先输出 parallel 步骤（讨论者），再输出 single 步骤（总结者）。
- 如果用户只 @ 一个 Agent 或没有明确顺序，用 single 步骤。

示例 1——三人讨论后 agent-a 总结：
用户请求：@agent-b @agent-c @agent-d 就方案A和方案B各自发表意见，然后 @agent-a 做裁决
输出：
{
  "steps": [
    {"type": "parallel", "agents": ["agent-b", "agent-c", "agent-d"], "instruction": "请就方案A和方案B发表你的观点和建议"},
    {"type": "single", "agents": ["agent-a"], "instruction": "请综合以上三位 Agent 的意见，给出最终裁决和行动建议"}
  ]
}

示例 2——单人直接回答：
用户请求：@agent-x 帮我分析这个需求
输出：
{
  "steps": [
    {"type": "single", "agents": ["agent-x"], "instruction": "请分析用户提交的需求"}
  ]
}

示例 3——全员讨论无总结：
用户请求：大家讨论一下这个技术方案
输出：
{
  "steps": [
    {"type": "parallel", "agents": ["<所有候选 Agent ID>"], "instruction": "请就该技术方案发表你的观点"}
  ]
}'
WHERE id = 'agent-system-planner';

-- ── 系统总结器（summarizer）────────────────────────────────────────────────────
INSERT OR IGNORE INTO agents (
    id,
    name,
    name_en,
    role,
    color,
    initial,
    llm_id,
    system_prompt,
    forge_role,
    role_type,
    system_kind,
    capabilities_json,
    max_concurrency,
    visible_in_chat,
    mentionable,
    enabled
) VALUES (
    'agent-system-summarizer',
    '总结器',
    'Summarizer',
    '多 Agent 讨论 · 总结裁决 · 行动建议',
    '#5a8a6f',
    '结',
    COALESCE(
        (SELECT id FROM llm_configs WHERE id = 'llm-sonnet4'),
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的系统总结器。在多 Agent 讨论结束后，把各方观点压缩成清晰、可执行、可追溯的最终结论。要求：综合各方观点而不照抄原文；若用户要求裁决则明确给出裁决和理由；输出后续行动建议；使用结构化 Markdown。',
    NULL,
    'system',
    'summarizer',
    '["summarizing","synthesis","adjudication"]',
    1,
    0,
    0,
    1
);

-- ── 文档生成器（doc_writer）───────────────────────────────────────────────────
INSERT OR IGNORE INTO agents (
    id,
    name,
    name_en,
    role,
    color,
    initial,
    llm_id,
    system_prompt,
    forge_role,
    role_type,
    system_kind,
    capabilities_json,
    max_concurrency,
    visible_in_chat,
    mentionable,
    enabled
) VALUES (
    'agent-system-doc-writer',
    '文档生成器',
    'Doc Writer',
    '群聊沉淀 · PRD · ADR · 测试计划',
    '#7a6faa',
    '文',
    COALESCE(
        (SELECT id FROM llm_configs WHERE id = 'llm-sonnet4'),
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的系统文档生成器。把群聊讨论沉淀为可引用、可迭代的正式文档产物（PRD、ADR、测试计划、实施方案等）。只输出 JSON，结构：{"kind":"...","title":"...","rows":[["key","val"]],"body":"Markdown 正文"}。不要遗漏背景、目标、范围、约束、风险和下一步。',
    NULL,
    'system',
    'doc_writer',
    '["documentation","prd","adr","spec"]',
    1,
    0,
    0,
    1
);

-- ── 上下文压缩器（context_compressor）────────────────────────────────────────
INSERT OR IGNORE INTO agents (
    id,
    name,
    name_en,
    role,
    color,
    initial,
    llm_id,
    system_prompt,
    forge_role,
    role_type,
    system_kind,
    capabilities_json,
    max_concurrency,
    visible_in_chat,
    mentionable,
    enabled
) VALUES (
    'agent-system-context-compressor',
    '上下文压缩器',
    'Context Compressor',
    '长对话压缩 · 摘要 · 降低 Token 消耗',
    '#6f8a9a',
    '压',
    COALESCE(
        (SELECT id FROM llm_configs WHERE id = 'llm-sonnet4'),
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的上下文压缩器。当群聊消息数超过窗口阈值时，将较早的消息压缩成可长期引用的结构化摘要。保留需求、决策、约束、待办、分歧和重要事实，删除寒暄和重复表达，用结构化 Markdown 输出，不编造未出现的信息。',
    NULL,
    'system',
    'context_compressor',
    '["compression","summarization","context_management"]',
    1,
    0,
    0,
    1
);
