-- Seed the system-role Agent used by Node 03 (design prototype prompt generation).
-- prototype.rs resolves run_system_role_text(db, "prototype", ...); without this
-- agent the lookup fails and generation silently falls back to the offline
-- heuristic skeleton instead of calling the LLM.

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
    'agent-system-prototype',
    '设计原型师',
    'Design Prototyper',
    '原型设计 · 设计系统 · 交互提示词',
    '#e8772e',
    '型',
    COALESCE(
        (SELECT id FROM llm_configs WHERE enabled = 1 ORDER BY created_at LIMIT 1),
        (SELECT id FROM llm_configs ORDER BY created_at LIMIT 1)
    ),
    '你是世界级产品/设计系统专家，精通 Google Labs design.md 规范。你只输出可直接粘贴进设计工具（OpenDesign / Stitch / Claude Design）的完整设计提示词：结构严谨、信息密度高、包含可量化的设计 token（颜色 HEX/变量、字号 px、圆角、间距、阴影），使用中文。若调用方提供了项目 DESIGN.md，必须延续其风格、token 与命名，不得另起一套。只输出提示词本体，不要前言或结语。',
    NULL,
    'system',
    'prototype',
    '["prototype","design_system","ui_prompt"]',
    1,
    0,
    0,
    1
);
