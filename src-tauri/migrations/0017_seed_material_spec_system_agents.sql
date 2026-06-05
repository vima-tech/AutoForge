-- Add system-role agents used by project material/spec AI commands.
-- These commands must resolve the currently assigned system-role Agent instead
-- of calling the local Claude CLI directly.

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
    'agent-system-material-ai',
    '物料助手',
    'Material AI',
    '物料库搜索 · 内容整理 · 分类建议',
    '#b97842',
    '物',
    COALESCE(
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1),
        (SELECT id FROM llm_configs ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的物料库 AI 助手。根据文件名、用途说明、标签、目录位置和内容片段完成语义检索与物料整理。严格遵守调用方要求的输出格式；要求 JSON 时只输出 JSON，不输出 Markdown，不补充解释。',
    NULL,
    'system',
    'material_ai',
    '["materials","semantic_search","organizing","classification"]',
    1,
    0,
    0,
    1
);

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
    'agent-system-spec-writer',
    '规格生成器',
    'Spec Writer',
    '项目规格 · 技术约束 · 测试要求',
    '#4f8ed1',
    '规',
    COALESCE(
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1),
        (SELECT id FROM llm_configs ORDER BY created_at LIMIT 1)
    ),
    '你是 AutoForge 的项目规格生成 Agent。根据项目信息、技术文件摘要和物料列表生成可执行、可维护的结构化规格约束。严格遵守调用方要求的输出格式；要求 JSON 时只输出 JSON，不输出 Markdown，不补充解释。',
    NULL,
    'system',
    'spec_writer',
    '["spec","technical_constraints","documentation"]',
    1,
    0,
    0,
    1
);
