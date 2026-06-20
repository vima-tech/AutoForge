# 待实现功能 / 技术债提案

本目录收录对 AutoForge 现有代码库做「空占位 / TODO / 未实现 / 残留 mock」全量梳理后，
**甄别为值得做**的功能提案。每份文档独立、自包含，含背景证据（file:line）、方案、验收与风险。

> **复评日期：2026-06-20**（与代码现状交叉核对后重整：删除已落地的提案，更新部分落地项的状态）。
> 核对方式：迁移文件实际序号、关键符号 grep、前端组件 import 真源比对。

## 当前有效提案

| 优先级 | 提案 | 状态 | 一句话 |
|--------|------|------|--------|
| P2 | [代码 Agent 可插拔](AutoForge-代码Agent可插拔-功能提案.md) | ✅ 已实现 | 抽 `CodeAgent` trait，claude/codex/opencode 配置驱动互换 + 全局默认/per-project 选择（2026-06-20 落地，迁移 0057，107 测试通过；任务清单 `代码Agent可插拔-tasks.json`） |
| P2 | [@提及/头像脱离 mock](AutoForge-提及与头像脱离Mock改用DB真源-功能提案-v2.md) | 📝 待实现 | `Avatar.tsx:19` / `Markdown.tsx:25` 仍 import `mock` 写死的 5 个 Agent，自建 Agent 的提及/头像渲染失败 |
| P3 | [Schema 驱动 Agent](schema-driven-agents.md) | 🟢 主体落地 | 脚手架+`agent_outputs`(0040)+analysis/test 样板已有；**本批(2026-06-20)新增**批量(1→N)解析、triage/proposer 升级为 schema 驱动并落库、字段级体检命令+Trace「schema 体检」面板（cargo test 104 passed）；剩版本 A/B·失败回灌·其余角色接入 |
| — | [脏输入压测 Dogfooding](dirty-input-dogfooding.md) | 🧪 方法论·待执行 | 把工厂往真实外包脏输入里摔、排序崩点 = 阶段二 roadmap；是验证流程而非单一代码特性 |

## 已完成（本次复评从清单移除其提案文档）

- ✅ **并发合并冲突解决**（迁移 `0050_merge_conflict`，`state::merge_lock` 同项目串行 + 合并前自动 merge-dev/rerere/重测，真冲突走 `merge_conflict` 态 + 可选 AI 解冲突回 review_2）。
- ✅ **操作者身份卡 + 通知中心**（迁移 `0042_notifications`，`App.tsx` rail-me 激活，`OperatorPanel.tsx` 身份卡 + 活动收件箱，事件持久化挂 `event::emit` 适配层）。
- ✅ **自定义 LLM 图片输入**（迁移 `0049_llm_supports_vision`，`llm.rs` 按 `supports_vision` 内联 base64 图片，未标记则静默丢图按纯文本，移除了旧一刀切硬报错）。
- ✅ **需求供给丝滑化 + AI 原生需求管理全案**（A 语音速录 `agents/asr.rs`+`QuickCapture.tsx`、B 捕获/分析解耦 `intake/gateway.rs`+`triage` 池、C 工厂自喂料 `tasks/autosupply.rs`+`intake/proposer.rs`、D 需求载体增强 bug 字段/`acceptance_json`+`cr_test_runs`；迁移 `0038_intake_triage`/`0039_cr_test_runs`）。
- ✅ **API Key 系统级加密落库**（`core/secrets.rs` 信封加密：主密钥进系统钥匙环，无钥匙环退化 0600 文件，密钥 AES-256-GCM 为 `enc:v1:` 密文落库）。

## 已甄别为「不值得成文」的命中（设计如此 / 正常状态）

- 端口 `{port}` 占位、`core/mask.rs` 掩码占位 —— 功能性占位，非待办。
- `intake/scanner.rs` 扫描 TODO/FIXME —— 这是「识别 TODO 作为需求入队」的功能本身。
- 各页 `尚未生成 / 尚未发布 / 暂无日志` —— 正常空态文案。
- 嵌入模型未配置时「dummy embedder / heuristic 召回」、部署/原型「无 LLM agent 走 heuristic」、ASR「未配置报错」、CR 预览「缺命令报错」—— 合理优雅降级/兜底，非半成品。
- MCP、会话归档、链路追踪、运行配置文件化、栈画像、配置备份 —— 均已完整实现，非占位。
