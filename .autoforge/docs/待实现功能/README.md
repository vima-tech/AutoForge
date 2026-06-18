# 待实现功能 / 技术债提案

本目录收录对 AutoForge 现有代码库做「空占位 / TODO / 未实现 / 残留 mock」全量梳理后，
**甄别为值得做**的功能提案。每份文档独立、自包含，含背景证据（file:line）、方案、验收与风险。

> 复评日期：2026-06-18（代码已迭代：文档阅读模式、长文档渲染、全链路加密/追踪、MCP 工具、
> 会话归档、运行配置文件化、ASR 语音录入、交付/原型/部署/评分/通知等模块）。
> 扫描范围：全仓 Rust + 前端 TS/TSX 的 TODO/FIXME/占位/降级/硬报错、后端 189 个注册命令 vs
> 前端 services 接线交叉核对、mock 数据生产泄露、CLAUDE.md 登记的延期项。

## 当前有效提案

| 优先级 | 提案 | 一句话 |
|--------|------|--------|
| P1 | [操作者身份卡 + 通知中心](AutoForge-操作者身份卡与通知中心-功能提案-v2.md) | 激活 `App.tsx:338` 静态 rail-me：操作者身份配置 + 把 13 类 AppEvent 中被丢弃的 9 类汇成活动收件箱 |
| P2 | [自定义 LLM 图片输入](AutoForge-自定义LLM图片输入支持-功能提案-v2.md) | 移除 `llm.rs:54-58` 对图片附件的一刀切硬报错，anthropic/openai 走多模态 |
| P2 | [@提及/头像脱离 mock](AutoForge-提及与头像脱离Mock改用DB真源-功能提案-v2.md) | `Markdown.tsx:25` / `Avatar.tsx:18` 改用 DB 真源，修自建 Agent 提及/头像渲染缺陷 |

## 已完成（从清单移除）

- ✅ **API Key 系统级加密落库**（2026-06-16 完成）—— `core/secrets.rs` 信封加密：主密钥进系统钥匙环
  （`keyring`，无钥匙环退化 0600 文件），LLM/MCP/web_search 密钥经 AES-256-GCM 加密为 `enc:v1:` 密文落 SQLite；
  `migrate_plaintext_secrets` 幂等搬迁；`secret_backend_status` 命令 + Settings 兜底 chip。CLAUDE.md 安全规则#6 / api.md 已同步。
  （此前的「迁移系统 keychain」提案已由该实现覆盖，故不再单列。）

## 已甄别为「不值得成文」的命中（设计如此 / 正常状态）

- 端口 `{port}` 占位、`core/mask.rs` 掩码占位 —— 功能性占位，非待办。
- `intake/scanner.rs` 扫描 TODO/FIXME —— 这是「识别 TODO 作为需求入队」的功能本身。
- 各页 `尚未生成 / 尚未发布 / 暂无日志` —— 正常空态文案。
- 嵌入模型未配置时「dummy embedder / heuristic 召回」—— Innate 的优雅降级，升级路径已实现（配置模型即可）。
- 部署/原型脚本「无 LLM agent 时走 heuristic」、ASR「未配置报错」、CR 预览「缺命令报错」—— 合理兜底，非半成品。
- `load_concurrency_settings` / `load_knowledge_settings` —— 前端用 get/set 等价命令，非缺口。
- MCP、会话归档、追踪、运行配置 —— 均已完整实现，非占位。
