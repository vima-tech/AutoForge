-- 捕获/分析解耦 + 需求载体增强。
-- triage（待整理）态复用 issues.status='triage'，不另起表；这些条目不自动进分析队列，
-- 由 triage Agent 炼成正经 issue 后才转 pending_analysis。
-- 其余列是「需求载体增强」：原始口述/碎片、Bug 复现要素、AI 验收标准。

ALTER TABLE issues ADD COLUMN raw_capture     TEXT;   -- 原始未整理文本（速录/语音/proposer 原话）
ALTER TABLE issues ADD COLUMN repro_steps     TEXT;   -- Bug 复现步骤
ALTER TABLE issues ADD COLUMN environment     TEXT;   -- Bug 环境（OS/版本/分支）
ALTER TABLE issues ADD COLUMN expected        TEXT;   -- Bug 期望结果
ALTER TABLE issues ADD COLUMN actual          TEXT;   -- Bug 实际结果
ALTER TABLE issues ADD COLUMN acceptance_json TEXT;   -- AI 生成的验收标准（JSON 数组，人审改）
