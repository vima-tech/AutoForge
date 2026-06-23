# AutoForge Agent 角色全景与规划 v1

> 本文梳理 AutoForge 当前**全部功能的 AI 触点**，给出**现有 Agent 角色清单**，
> 重点做**缺口分析**，并提出一套**完整、分阶段的 Agent 角色规划**。
> 真源以代码为准（`src-tauri/src/agents/`、`tasks/`、`commands/`、`migrations/`）。

---

## 0. 角色机制速览

AutoForge 的 Agent 分两个维度：

- **`role_type`**：`business`（会议室可见、可 @）/ `system`（隐藏，系统按职责调用）。
- **职责绑定**：
  - **`system_kind`**（逗号分隔，一个 Agent 可兼任多个）——由 `run_system_role_text(db, kind, …)` 解析「enabled 且含该 kind」的第一个 Agent，用其**绑定的 LLM**。
  - **`forge_role`**（逗号分隔，`analysis` / `test`）——流水线阶段绑定，在「角色指派 → 流水线角色」里设置。
- **执行通道**：
  - **本地 `claude` CLI**：仅**代码实现（Claude Code）** + 需求消毒兜底 + CLI 健康探测。
  - **自定义 LLM API**：其余所有角色走各自绑定的 `llm_configs`（Anthropic / OpenAI 兼容 / Ollama）。
  - 成本策略：**除 Claude Code 外，全部走可配置自定义 LLM 以省钱**。

---

## 1. 系统功能全景 · AI 触点地图

| # | 功能域 | 入口/触发 | 代码位置 | AI 触点 | 当前驱动角色 |
|---|--------|-----------|----------|---------|--------------|
| 1 | 需求接收 | Webhook / GitHub / 扫描 / 批量 / 手动 | `commands/intake.rs`、`IntakePanel` | 无（仅入库） | — |
| 2 | 输入安全消毒 | 分析前（外部来源） | `tasks/analysis.rs:38` `safety_check` | ✅ LLM 判定注入 | ⚠️ 硬编码 Claude CLI，**无角色** |
| 3 | 需求分析 | Analysis 任务 | `agents/analysis.rs::analyze` | ✅ 结构化分析（真实性/可行性/scope/计划/AC） | **`analysis`**（forge_role） |
| 4 | 审核 1（需求） | 功能审计页 `review1` | `commands/change_requests.rs` | 人工 | —（人类节点） |
| 5 | 代码实现 | Execution 任务（worktree） | `agents/code_agent.rs` | ✅ Claude Code | **Claude Code / `coder`**（CLI，故意保留） |
| 6 | 风险分级 | 实现后 | `agents/grader.rs` | ✅ 启发式 + LLM 兜底评 T0–T3 | **`grader`** |
| 7 | 门控降级（自动放行） | 实现后 | `core/gate.rs` | 规则（无 LLM） | — |
| 8 | 审核 2（代码） | 功能审计页 `review2` | `commands/change_requests.rs` | 人工 + diff/报告/预览 | —（人类节点） |
| 9 | 合并 | review2 通过 | `tasks/merge.rs` | git（无 LLM） | — |
| 10 | 合并后测试 | Merge 后 | `tasks/testing.rs` | ❌ 仅跑 shell 命令 | ⚠️ `test`（forge_role）**已指派但未接 LLM** |
| 11 | 合并后安全审查 | Merge 后 | `tasks/security_audit.rs` | ✅ 审查 diff，高危回填需求 | **`security`** |
| 12 | 主动巡检 | 定时/手动 | `tasks/scan.rs` | 扫描 + 入队（无 LLM） | — |
| 13 | 原型设计提示词 | 交付·设计 | `commands/prototype.rs` | ✅ 生成设计提示词 | **`prototype`** |
| 14 | 部署脚本生成 | 交付·部署 | `commands/deploy.rs` | ✅ 生成部署脚本 | **`deploy`** |
| 15 | 交付产物 / Widget | 交付·归档 | `commands/artifacts.rs`、`widget.rs` | 无 | — |
| 16 | 通知通道 | 事件触发 | `core/notify.rs` | 无（仅格式化推送） | — |
| 17 | 物料库 AI | 项目管理·物料 | `commands/materials.rs` | ✅ 语义搜索 / 整理 | **`material_ai`** |
| 18 | 规格生成 | 项目管理·规格 | `commands/specs.rs` | ✅ 生成技术约束 | **`spec_writer`** |
| 19 | 群聊编排 | 会议室任务 | `commands/orchestration.rs` | ✅ 计划/讨论/总结/文档/压缩 | **`planner`/`summarizer`/`doc_writer`/`context_compressor`** + 业务 Agent |
| 20 | 知识库 | kb_recall/add/evolve | `knowledge/mod.rs` | ⚠️ 已有存取，**无角色驱动萃取** | —（缺角色） |

---

## 2. 当前 Agent 角色清单（共 12 类已落位）

### 2.1 业务 Agent（`role_type=business`，会议室可见 · 播种于 `0002_seed.sql`）

| 名称 | ID | forge_role | 默认 LLM | 职责 |
|------|----|-----------|---------|------|
| 需求分析师 | `agent-analyst` | analysis | opus4 | 真实性/可行性/优先级/查重/实现计划 |
| Claude Code | `agent-coder` | — | sonnet4 | worktree 内代码实现 + 报告 |
| 测试工程师 | `agent-tester` | test | sonnet4 | （当前仅作 chat 角色，未驱动测试 LLM） |
| 架构顾问 | `agent-architect` | — | opus4 | 技术选型、模块设计、重构建议 |
| 安全审计 | `agent-security` | — | sonnet4 | chat 中的安全讨论 |

### 2.2 系统角色（`role_type=system` · `system_kind`）

| system_kind | 名称 | 播种？ | 驱动功能 |
|-------------|------|--------|----------|
| planner | 调度器 | ✅ | 群聊编排计划 |
| summarizer | 总结器 | ✅ | 群聊综合/裁决 |
| doc_writer | 文档生成器 | ✅ | PRD/ADR/测试计划 |
| context_compressor | 上下文压缩器 | ✅ | 长对话压缩 |
| material_ai | 物料助手 | ✅ | 物料搜索/整理 |
| spec_writer | 规格生成器 | ✅ | 项目规格生成 |
| grader | 风险分级器 | ❌（需手动建/指派） | diff 风险 T0–T3 |
| security | 安全审查 | ❌ | 合并后安全审查 |
| deploy | 部署脚本 | ❌ | 部署脚本生成 |
| prototype | 设计原型师 | ❌（用户已手动建） | 原型设计提示词 |

> 角色指派 UI（设置 → 角色指派）已覆盖以上全部 10 个 system_kind + analysis/test 两个 forge_role。

---

## 3. 缺口分析（重点）

### 3.1 "假开关" / 未接通

| 问题 | 现状 | 影响 | 建议 |
|------|------|------|------|
| **`test` 未接 LLM** | 测试任务只跑 shell（unit/integration/quality 命令），`test` forge_role 指派后不影响任何 LLM | "测试工程师"形同虚设；测试失败无 AI 诊断 | 新增 **测试设计/失败诊断** 角色（见 4） |
| **消毒无角色** | `safety_check` 硬编码 Claude CLI，烧 Claude 额度且不可换模型 | 与"省钱"策略冲突、不可配置 | 抽出 **`sanitizer` 输入安全官** 角色，走自定义 LLM |
| **4 个角色未播种** | grader/security/deploy/prototype 无默认 Agent | 全新环境首用即报"未配置系统角色" | 补 seed 迁移（`INSERT OR IGNORE`） |

### 3.2 流程缺口（缺少的能力）

| 缺口 | 说明 | 优先级 |
|------|------|--------|
| **代码评审（Code Reviewer）** | 审核 2 前无 AI 预审；grader 只评级不读代码。应在实现后自动出"评审意见"附给人类 | **P0** |
| **测试作者（Test Author）** | 把 analysis 的 `test_plan` 落成真实测试代码 / 失败时定位根因 | **P0** |
| **需求澄清（Clarifier）** | analysis 产出 `open_questions`，目前全靠人在审核 1 回答；可整理澄清清单/回问提交者 | P1 |
| **发布说明（Release Notes）** | 合并时无变更说明 / CHANGELOG 生成 | P1 |
| **知识策展（Knowledge Curator）** | `knowledge` 模块有存取，但无角色从已合并 CR 萃取经验/规范回灌 | P1 |
| **文档维护（Docs Maintainer）** | 合并后不更新 README/CLAUDE.md/specs | P2 |
| **依赖治理（Dependency Steward）** | scan 只 audit，无 AI 规划升级/改 lockfile | P2 |
| **设计走查（Design QA）** | 实现后无 AI 核对是否符合 DESIGN.md / 原型 | P2 |
| **Backlog 排程（Planner/Triager）** | 队列按 analysis 给的静态 priority 排，无动态重排/容量规划 | P2 |

---

## 4. 完整 Agent 角色规划（目标蓝图）

> 标注：✅已有 · ⚠️假开关需接通 · ➕新增。`type` = system_kind（除非注明 forge_role）。

### 阶段 A · 接收与分诊

| 角色 | type | 状态 | 职责 | 输入 → 输出 | LLM 分层 |
|------|------|------|------|------------|----------|
| 输入安全官 Sanitizer | `sanitizer` | ➕ | 注入/越权/敏感检测，替代硬编码 safety_check | 需求文本 → 通过/拒绝+理由 | 便宜/本地 |
| 需求分析师 Analyst | `analysis`(forge) | ✅ | 真实性/可行性/scope/实现计划/AC | 需求+项目上下文 → 结构化分析 | 强模型 |
| 需求澄清官 Clarifier | `clarifier` | ➕ | 把 open_questions 整理成澄清清单/回问 | analysis → 澄清问题 | 中等 |
| 去重分诊 Deduper | `dedup` | ➕(可并入 analysis) | 跨需求查重、合并建议 | 需求+历史 → 重复判定 | 便宜 |

### 阶段 B · 实现与质量

| 角色 | type | 状态 | 职责 | LLM 分层 |
|------|------|------|------|----------|
| Claude Code | `coder` | ✅ | worktree 内实现（**唯一用 Claude CLI**） | Claude CLI |
| 代码评审 Reviewer | `code_reviewer` | ➕ **P0** | 读 diff 出评审意见（bug/规范/坏味道），喂给审核 2 | 强模型 |
| 风险分级 Grader | `grader` | ✅ | diff → T0–T3 + 变更类 | 中等 |
| 测试作者 Test Author | `test_author` | ➕ **P0**（接 `test`） | 生成/补全测试用例、失败根因诊断 | 强/中 |
| 安全审查 Security | `security` | ✅ | 合并后安全审查，高危回填 | 中等 |
| 设计走查 Design QA | `design_qa` | ➕ | 核对实现 vs DESIGN.md/原型 | 中等 |

### 阶段 C · 交付与运维

| 角色 | type | 状态 | 职责 | LLM 分层 |
|------|------|------|------|----------|
| 设计原型师 Prototyper | `prototype` | ✅ | 原型设计提示词 | 中等 |
| 部署官 Deployer | `deploy` | ✅ | 部署脚本生成 | 中等 |
| 发布说明 Release Notes | `release_notes` | ➕ | 合并→变更说明/CHANGELOG | 便宜 |
| 依赖治理 Dependency Steward | `dependency` | ➕ | 依赖升级规划、lockfile 改动 | 中等 |
| 通知格式化 Notifier | `notifier` | ➕(可选) | 把事件润色成人话推送 | 便宜/本地 |

### 阶段 D · 协作与知识（会议室 + 知识库）

| 角色 | type | 状态 | 职责 | LLM 分层 |
|------|------|------|------|----------|
| 调度器 Planner | `planner` | ✅ | 群聊编排计划 | 中等 |
| 总结器 Summarizer | `summarizer` | ✅ | 综合/裁决 | 中等 |
| 文档生成器 Doc Writer | `doc_writer` | ✅ | PRD/ADR/测试计划 | 中等 |
| 上下文压缩器 Compressor | `context_compressor` | ✅ | 长对话压缩 | 便宜/本地 |
| 物料助手 Material AI | `material_ai` | ✅ | 物料搜索/整理 | 便宜 |
| 规格生成器 Spec Writer | `spec_writer` | ✅ | 项目技术规格 | 中等 |
| 架构顾问 Architect | business | ✅ | chat 技术咨询 | 强模型 |
| 知识策展 Knowledge Curator | `knowledge_curator` | ➕ | 从已合并 CR 萃取经验/规范回灌知识库 | 中等 |
| 文档维护 Docs Maintainer | `docs_maintainer` | ➕ | 合并后更新 README/CLAUDE.md/specs | 中等 |

---

## 5. LLM 分层与成本建议

| 层 | 用途 | 角色 |
|----|------|------|
| **Claude CLI** | 需工具执行/写文件 | 仅 `coder`（Claude Code） |
| **强模型**（贵，准确性关键） | 决策质量直接影响成败 | analysis、code_reviewer、grader(兜底)、architect、test_author |
| **中等模型** | 生成类、结构化 | doc_writer、spec_writer、prototype、deploy、security、planner、summarizer、design_qa、clarifier、dependency、knowledge_curator |
| **便宜/本地模型**（省钱） | 判定/压缩/格式化 | sanitizer、context_compressor、material_ai、dedup、release_notes、notifier |

---

## 6. 落地优先级与建议

1. **P0 · 接通缺口**
   - 新增 **`code_reviewer`（代码评审）** — 审核 2 前自动出评审意见，最大提升人审效率与质量。
   - 接通 **`test_author`** 并把 `test` forge_role 真正绑到 LLM（当前测试只跑 shell）。
   - 抽出 **`sanitizer`** 取代硬编码 `safety_check`，走自定义 LLM。
2. **P0 · 可复现性**
   - 补一条 **seed 迁移**：为 `grader / security / deploy / prototype`（及上面新增角色）各建一个默认 `INSERT OR IGNORE` 系统 Agent，全新环境开箱即用。
3. **P1**：`clarifier`、`release_notes`、`knowledge_curator`。
4. **P2**：`docs_maintainer`、`dependency`、`design_qa`、`notifier`、backlog 排程。

> 每新增一个 system_kind 角色，落地清单：① seed 迁移建默认 Agent；② 在调用处用 `run_system_role_text(db, "<kind>", …)`；③ 「角色指派」UI 增加对应行（`Settings.tsx` 的 `orchestrationRows`/`deliveryRows`）；④ 默认绑定便宜/强模型按上表分层。
