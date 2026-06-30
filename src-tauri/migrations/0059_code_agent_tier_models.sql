-- 分级选模型：给每个 code agent 配「快模型 / 强模型」。
-- 执行前用分析阶段的风险信号（blast_radius + 影响文件数 + 复杂度）挑选：
--   低风险 → fast_model，否则 → strong_model；二者为空时回落到 model（原行为不变）。
-- 三家（claude / codex / opencode）共用同一机制，仅模型名各填各的。
ALTER TABLE code_agents ADD COLUMN fast_model   TEXT;
ALTER TABLE code_agents ADD COLUMN strong_model TEXT;
