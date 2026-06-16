-- 接口规范收敛为仅两种：openai、anthropic（先支持这两种 HTTP wire 格式）。
-- 0031 回填可能产生 ollama / claude-cli 等历史值，统一归一到 openai（“未知回退 openai”）。
-- 本地 Claude CLI 不再作为 api_spec 表达：未绑定 LLM（llm_id IS NULL）的 Agent 仍走本地 CLI。
UPDATE llm_configs SET api_spec = 'openai'
 WHERE api_spec NOT IN ('openai', 'anthropic');
