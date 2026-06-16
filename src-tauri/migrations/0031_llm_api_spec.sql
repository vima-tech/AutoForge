-- 显式声明每个 LLM 配置的接口规范（wire spec），供工具调用按格式选择。
-- 文本生成可靠子串推断 provider 即可，但工具调用的 OpenAI 与 Anthropic
-- 格式不可互换（tools 声明 / tool_calls vs tool_use / 结果回灌方式均不同），
-- 故必须显式声明，不再靠 provider 名称猜测。
--   openai     → /v1/chat/completions，tools=[{type:function,...}]，message.tool_calls
--   anthropic  → /v1/messages，tools=[{name,input_schema}]，content[].type=tool_use
--   ollama     → /api/chat，OpenAI 风格 tools（支持度参差，运行时优雅降级）
--   claude-cli → 本地 CLI，自带工具体系，自定义工具循环不适用
ALTER TABLE llm_configs ADD COLUMN api_spec TEXT NOT NULL DEFAULT 'openai';

-- 按既有 provider 关键字回填，与 agents/llm.rs 的路由保持一致。
UPDATE llm_configs SET api_spec = 'claude-cli' WHERE lower(provider) LIKE '%claude-cli%';
UPDATE llm_configs SET api_spec = 'anthropic'  WHERE lower(provider) LIKE '%anthropic%' AND api_spec = 'openai';
UPDATE llm_configs SET api_spec = 'ollama'     WHERE lower(provider) LIKE '%ollama%'    AND api_spec = 'openai';
