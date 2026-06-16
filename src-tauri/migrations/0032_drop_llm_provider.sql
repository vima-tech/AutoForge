-- 移除 llm_configs.provider：与 0031 引入的 api_spec 语义重叠、易歧义。
-- api_spec（openai|anthropic|ollama|claude-cli）成为接口规范的唯一真源，
-- 同时驱动文本生成路由与工具调用 wire 格式。0031 已先按 provider 回填 api_spec，
-- 故此处丢弃 provider 不损失信息。provider 有默认值且无索引引用，可直接 DROP COLUMN。
ALTER TABLE llm_configs DROP COLUMN provider;
