# AutoForge 角色 Agent 完整设计 v1

> 本文是 AutoForge「角色 Agent」体系的完整设计与实现说明，配套代码真源：
> `src-tauri/src/agents/roles.rs`（内置注册表 + 基线提示词）、`agents/llm.rs`（提示词拼接）、
> `commands/settings.rs`（角色目录/配置命令）、`src/pages/Settings.tsx`（「角色」页 UI）。

---

## 1. 设计目标

把原先「LLM 配置 / Agent 配置 / 角色指派」**三层三页**精简为 **两层**：`LLM 配置` + `角色`。

- **角色即 Agent**：每个职责槽位就是一个能干活的 Agent，不再「先建通用 Agent 再去指派」。
- **开箱即用**：每个系统角色自带**专业优化的内置提示词**，选一个 LLM 即可启用，默认零配置。
- **可补充可自定义**：提示词支持「内置 / 内置+补充 / 完全自定义」三种模式。
- **省钱**：除 Claude Code（代码实现，本地 `claude` CLI）外，所有角色走各自绑定的自定义 LLM。
- **一角色一 Agent**：不支持一个 Agent 兼多个角色，保证清晰。
- **统一聊天能力**：所有角色都可选择「可拉入群聊 / 可私聊」。

---

## 2. 两层模型

```
第 1 层  LLM 配置        provider / model / key / endpoint        （不变）
第 2 层  角色 Role-Agent
   ├─ 系统角色（内置目录，来自 ROLE_REGISTRY）
   │    分三组：群聊编排 / 交付与项目 / 需求流水线
   │    卡片：启用 · 选 LLM · 提示词模式[内置/内置+补充/自定义] · 群聊/私聊开关
   └─ 对话角色（用户自定义业务 Agent，群聊/私聊用）
        无限创建：名字/头像 · LLM · 提示词(模板或自由) · 可@ · capabilities
```

---

## 3. 角色机制

### 3.1 关键字段（`agents` 表）

| 字段 | 含义 |
|------|------|
| `role_type` | `system` / `business`（对话角色） |
| `system_kind` | 系统角色标识（逗号分隔字段，但本设计**单一持有**） |
| `forge_role` | 流水线角色标识：`analysis` / `test` |
| `llm_id` | 绑定的 LLM 配置 |
| `system_prompt` | 提示词的「补充」或「自定义」文本（语义由 `prompt_mode` 决定） |
| `prompt_mode` | `builtin` / `append` / `custom`（迁移 `0027`，默认 `builtin`） |
| `visible_in_chat` | 可私聊（自动建直聊） |
| `mentionable` | 可拉入群聊、可 @ |
| `enabled` | 是否启用 |

### 3.2 提示词拼接（`roles::compose_system_prompt`）

```
final = builtin                              当 mode='builtin'
final = builtin + "\n\n# 补充约束\n" + 文本   当 mode='append'
final = 自定义文本（空则退回 builtin）          当 mode='custom'
final = 自定义文本                            当无注册表项（纯对话角色）
```

`run_system_role_text(db, kind, …)` 解析持有该 kind 的 enabled Agent → 按 `prompt_mode` 组合 → 调其绑定 LLM。注册表无该 kind 或组合为空时退回调用方传入的兜底提示词。

### 3.3 执行通道

| 通道 | 用途 | 角色 |
|------|------|------|
| 本地 `claude` CLI | 需工具执行/写文件 | **仅 Claude Code（代码实现）** |
| 自定义 LLM API | 其余全部 | 所有系统角色 + 对话角色（Anthropic / OpenAI 兼容 / Ollama） |

---

## 4. 完整角色清单（12 个内置 + 自定义对话角色）

> binding：`system_kind` 或 `forge_role`。默认聊天：是否默认开启群聊/私聊。

### 4.1 群聊编排组（orchestration）

| kind | 名称 | binding | 职责 |
|------|------|---------|------|
| `planner` | 调度器 Planner | system_kind | 自然语言请求 → 多 Agent 并发/串行编排计划（JSON，含 few-shot） |
| `summarizer` | 总结器 Summarizer | system_kind | 多 Agent 讨论后综合、裁决、行动项 |
| `doc_writer` | 文档生成器 Doc Writer | system_kind | 沉淀 PRD/ADR/方案/测试计划（结构化 JSON） |
| `context_compressor` | 上下文压缩器 | system_kind | 长对话压缩为信息无损结构化摘要 |

### 4.2 交付与项目组（delivery）

| kind | 名称 | binding | 职责 |
|------|------|---------|------|
| `grader` | 风险分级器 Grader | system_kind | diff 评 T0–T3，决定能否门控自动放行 |
| `security` | 安全审查 Security | system_kind | 合并后审查 diff，高危回填为需求 |
| `prototype` | 设计原型师 Design Prototyper | system_kind | 生成可粘贴进设计工具的原型提示词 |
| `deploy` | 部署官 Deployer | system_kind | 按目标环境生成 bash 部署脚本 |
| `material_ai` | 物料助手 Material AI | system_kind | 物料语义检索与整理 |
| `spec_writer` | 规格生成器 Spec Writer | system_kind | 生成项目技术规格约束 |

### 4.3 需求流水线组（pipeline）

| kind | 名称 | binding | 职责 | 默认聊天 |
|------|------|---------|------|----------|
| `analysis` | 需求分析师 Analyst | forge_role | 审核 1 前结构化分析（真实性/可行性/scope/计划/AC） | 是 |
| `test` | 测试工程师 Test Engineer | forge_role | 设计测试用例 + 失败诊断 | 是 |

### 4.4 对话角色（自定义）

用户自由创建的业务 Agent，无 kind、无内置提示词（提示词即其自身文本）。用于群聊 @ / 私聊 / 被 Planner 编排 / 写 `.autoforge` 工作区。提供起手模板：通用助手 / 评审 / 文案 / 分析。

---

## 5. 各角色内置提示词（专业基线）

> 以下为 `roles.rs` 中的 `builtin_prompt`，`prompt_mode='builtin'` 时即生效。
> `analysis` 复用 `agents/analysis.rs::SYSTEM_PROMPT`（完整 JSON schema，篇幅大，见代码）。

### Planner 调度器
> 系统调度 Agent：把群聊自然语言请求转为严格 JSON 编排计划。`parallel`（并发讨论）/`single`（总结裁决）。只输出 `{"steps":[{"type","agents","instruction"}]}`，不虚构 Agent ID。含 3 个 few-shot 示例（三人讨论后总结 / 单人回答 / 全员讨论）。

### Summarizer 总结器
> 总结裁决官：综合各方观点、提炼共识与分歧、给出裁决与理由、输出行动项。结构化 Markdown 分节（结论/共识/分歧/裁决/行动项），客观可追溯，不臆造。

### Doc Writer 文档生成器
> 把讨论沉淀为正式文档。**只输出 JSON** `{"kind","title","rows":[["字段","值"]],"body":"Markdown"}`；body 必含背景/目标与非目标/范围/方案或决策(ADR含被否方案)/约束/风险/验收/下一步；信息不足标「待确认」。

### Context Compressor 上下文压缩器
> 把较早消息压成信息无损结构化摘要。保留需求/决策/约束/未决/待办/关键事实，删寒暄与重复；分节 Markdown，优先保真。

### Material AI 物料助手
> 物料语义检索与整理：相关性排序有据、分类命名遵循项目习惯、不臆测；严格遵守调用方指定输出格式（要 JSON 即只输出 JSON）。

### Spec Writer 规格生成器
> 生成可执行可校验的规格约束（技术栈/架构/编码/API/测试）；具体不空泛、与技术栈一致、不矛盾；严格遵守调用方指定格式。

### Grader 风险分级器
> 评 diff 合并风险，**只输出 T0/T1/T2/T3**。T0 文档/格式/纯测试；T1 局部小逻辑；T2 常规业务/跨文件；T3 schema/迁移/auth/支付/安全/依赖/公共契约/大爆炸半径。就高不就低，不确定从高。

### Security 安全审查
> 只报真实可利用问题（密钥泄露/注入/越权/不安全反序列化/路径穿越/SSRF/危险依赖/敏感信息明文/加密误用）。**严格输出 JSON 数组** `[{"severity","title","detail"}]`，无问题输出 `[]`。

### Prototype 设计原型师
> 世界级设计系统专家，输出可粘贴进设计工具的完整提示词：设计目标/气质、设计 token（HEX/px/间距/圆角/阴影）、布局栅格断点、逐屏状态、组件与无障碍；量值给具体数值或 token 名；复用 DESIGN.md；只输出提示词本体。

### Deploy 部署官
> 生成 bash 部署脚本：`set -euo pipefail`、幂等、覆盖依赖/构建/迁移/发布或重启/健康检查/回滚；变量集中、危险操作前校验。**只输出纯 bash，无代码围栏**。

### Test 测试工程师
> 依据 test_plan/验收/改动设计可执行用例（正常/边界/异常/回归），失败时定位根因+最小复现+修复方向；聚焦行为、不写无意义用例；遵守调用方格式。

---

## 6. 后端实现

| 文件 | 作用 |
|------|------|
| `migrations/0027_agent_prompt_mode.sql` | 给 `agents` 加 `prompt_mode`（默认 `builtin`） |
| `agents/roles.rs` | `ROLE_REGISTRY`（12 条 `RoleDef`）+ 内置提示词 + `compose_system_prompt` |
| `agents/llm.rs` | `run_system_role_text` 按 `prompt_mode` 组合提示词 |
| `agents/analysis.rs` | `analyze()` 用 analysis Agent 绑定 LLM（CLI 仅兜底）；`SYSTEM_PROMPT` 已 pub 供注册表引用 |
| `commands/settings.rs` | `list_role_catalog`（注册表 join 持有 Agent）、`set_role_slot`（建/改单一持有 Agent） |

`set_role_slot(kind, payload)`：找该 kind 的持有者；无则按注册表默认创建专属 Agent；清除其它持有者保证**单一持有**；应用 `llm_id / prompt_mode / supplement / enabled / visible_in_chat / mentionable`。

---

## 7. 前端「角色」页（`Settings.tsx`）

- **入口**：设置导航单项「角色」（已合并旧「Agent 配置 + 角色指派」）。
- **结构**：三个系统角色分组面板 + 一个对话角色面板，**面板默认收起**，点标题展开。
- **分组面板头**：标题 + 完整配置计数 `已配全/总数`（全配为绿、有缺为琥珀并带 ⚠）+ 展开箭头。「完整配置」= 有持有 Agent **且** 已启用 **且** 已绑 LLM。
- **角色卡**：**默认折叠**，折叠态显示 `当前 LLM · 提示词模式` + 群/私小标签 + 状态 chip（已启用/已停用/缺 LLM/未配置）；展开后维护：LLM 选择、提示词模式 seg、补充/自定义文本框、群聊/私聊/启用开关。
- **对话角色**：卡片列表 + 「新建对话角色」，含起手模板，同样有群聊/私聊开关。

---

## 8. 角色在工作流中的位置

```
需求接收 →（sanitizer 消毒*）→ analysis 分析 →[审核1]→ Claude Code 实现
        → grader 评级 →（门控降级?）→[审核2]→ 合并
        → 合并后：test 测试 · security 安全审查 →（缺陷回填需求，回到分析）
交付：prototype 原型 · deploy 部署
项目：material_ai 物料 · spec_writer 规格
群聊：planner 编排 → 业务/对话角色讨论 → summarizer 总结 → doc_writer 出文档（context_compressor 压缩历史）
（* sanitizer 为规划中角色，详见《Agent 角色全景与规划》）
```

---

## 9. 迁移与兼容

- 迁移 `0027` 只加列、默认 `builtin`；现有系统角色 Agent 在 builtin 模式下采用升级后的注册表提示词。
- 现有业务 Agent → 自动归入「对话角色」（无 kind → 用自身提示词）。
- 单一持有由 `set_role_slot` 保障；`grader/security/deploy/prototype` 等首次在角色卡选 LLM 即创建持有 Agent（亦可由 seed 迁移预置）。

---

## 10. 关联文档

- 角色缺口与规划：`AutoForge-Agent角色全景与规划-v1.md`
- 实施方案：`AutoForge-Agent三层精简两层-实施方案-v1.md`
