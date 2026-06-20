-- 标记 LLM 是否支持多模态（图片识别）。默认 0（仅文本），用户在 LLM 配置中按模型能力显式开启。
-- 开启后，绑定该 LLM 的角色 Agent 在会议室中可接收并识别图片附件（见 agents/llm.rs run_agent_text）。
ALTER TABLE llm_configs ADD COLUMN supports_vision INTEGER NOT NULL DEFAULT 0;
