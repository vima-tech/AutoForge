-- 群聊成员后端化：agents.code_agent_id 非空 ⇒ 该成员由编码 CLI（claude/codex）只读跑项目
-- 仓库作答（会议室答疑、不写盘）；为空 = 现状 LLM 后端（零回归）。
-- 与 projects.code_agent_id（项目级「执行」覆盖）同名但属不同表、不同语义：此处是「会议室成员
-- 的回复引擎」，那里是「代码实现任务用哪个 CLI」。沿用裸 TEXT、不加 FK（code_agents 行被删时
-- 运行期按「查不到→降级」处理，见 agents::code_agent::resolve_by_id）。
ALTER TABLE agents ADD COLUMN code_agent_id TEXT;
