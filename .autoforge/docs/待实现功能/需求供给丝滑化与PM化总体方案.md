# 需求供给丝滑化 + 项目管理工具化 — 总体方案

> 状态：方案评审中（2026-06-17）
> 已定决策：① ASR 配置 = Settings 新增独立 section「语音录入」(mic)；
> ③ proposer = 工程视角为主（带 file:line 证据），可附带少量高优先级强烈建议的新功能；
> ④ 测试 = AI 生成验收标准（人审改）+ review_2 整体自动测试，不做人工用例库/逐用例执行；
> ⑤【定调】本系统是**软件工厂**，不是 AI 项目管理工具。判据：每个特性必须"给机器更好的料 /
>    让机器自检产出"，**不得**变成"给人管理工作的台子"。据此 D 阶段砍掉：测试用例库页面、
>    模块树、迭代/Sprint（纯 PM 官僚）。保留：bug 字段（需求载体增强）+ AI 验收标准 + CR 级遥测。
> 目标：解决"原材料（需求）供给不丝滑、不稳定、不枯竭"两大痛点，并把系统抬到禅道级的
> bug / 测试用例管理能力。
> 约束：严格遵守 `CLAUDE.md` 铁律——业务逻辑零 Tauri 依赖、事件只走 `event::emit`、
> 命令薄包装、迁移仅追加、IPC 走 services 层、CSS 只用变量、禁原生 `<select>`。

---

## 北极星：需求是一条"传送带"，不是一张"列表/看板"

整套系统的统一心智模型（一切取舍的最终裁判）：

> **需求是一条会自己流动、自己排序、自己标注的传送带。**
> 人只站在旁边做两个动作：**往上扔念头**（捕获），**在闸口点头或摇头**（裁决）。
> 中间的一切——结构化、去重、排序、状态、路由、实现、自检——都是机器的事。

由此推出的硬约束（凡违背即为退化成 PM 工具，见 §0.1）：

- **状态靠观测，不靠人更新**：需求状态从流水线推导（有无 CR / 是否合并 / 测试结果），
  不存在"人把卡从进行中拖到完成"的动作。
- **优先级靠涌现，不靠人拖拽**：由证据算出且可解释，backlog 自排序。
- **结构靠推断，不靠人填**：分类/严重度/模块/重复指向都是机器推断的活属性。
- **供给靠自繁殖，不只靠人写**：proposer/扫描持续产料，解决"原料稳定供应"。
- **界面形态向"传送带"靠拢，远离"列表+看板"**：任何"人来组织工作"的视图都是警报。

稀缺资源已翻转：不再是"写/理需求的人力"，而是**"决定不做什么"**（防需求通胀）
与**"让工厂瞄准真实意图"**（防价值漂移）。两道审核闸正是这两个稀缺资源的落点。

### 信任是渐变的：传送带也有"控制室 + 节流阀"

"只剩两个手势"是**终态**，不是起点。现阶段对 AI 尚未完全放心，需要**强的全局把控感**——
但要分清两种把控，一种该有、一种是 PM 工具回潮：

- **观测性把控（看得见 / 能急停 / 能调速）= 玻璃墙 + 急停 + 节流阀。看 ≠ 管。** ✅ 现阶段拉满。
- **管理性把控（被迫组织工作：拖优先级 / 改状态 / 指派 / 规划迭代）= PM 台子。** ❌ 仍然不做。

> 低信任档的传送带，不是退回看板，而是**多装检查点 + 一整面玻璃墙**；人仍只做"看 / 否"，不做"组织"。
> 与 PM 工具的本质区别始终是：**监视与否决，而非编写与梳理。**

**信任 = 旋钮，不是开关**。引入可调 **autonomy level**（复用现有 gating + 并发流控 + 双闸抽象）：
- *最紧档（现在）*：proposer 关、triage 全人工过、零自动合并、小批量、每个 AI 决策摊开推理。
- *放宽*：开 proposer、增大批量、低风险自动跑。**"扔/点头"两个手势的数量本身随旋钮变化。**

把控感的现成基建：
- **控制室总览**：一屏看整条传送带 + 在途需求 + 工厂"正在想什么/打算做什么"；每个 AI 决策可点开
  看推理——**这是 `llm_traces`（链路追踪）基建的价值兑现，把控感主体即可观测性**。
- **随处急停/接管**：任意一格可暂停/改/否/退回，是权利非义务（不点则照流）。
- **信任旋钮**：gating 降级 + 并发流控 + review_1/2 抽象成一个 autonomy level。

正反馈：**把控感做对了，正是信任的引桥**——反复看见推理、看见它做对，才敢把旋钮往上拧；
墙渐薄、闸渐宽，系统自然滑向"只剩两个手势"的终态。观测性不是对北极星的妥协，是通往它的路。

---

## 0. 诊断与设计哲学

需求供给的"不丝滑"沿两条**不同的轴**，必须分开治理：

- **轴 A — 捕获成本（你*有*料时）**：现有 6 通道都要求交出"已成型需求"（标题+分类+严重度），
  把人当成了质检员。病根是**捕获与加工焊死**。治法：捕获零结构、加工交给已有 analysis Agent。
- **轴 B — 自给能力（你*没*料时）**：扫描器是唯一能自产料的，但**无调度**（`run_code_scan`
  仅手动命令），且一次扫完即枯竭。治法：周期调度 + 提议型 Agent，让工厂自己找活。

PM 化（禅道级）则是第三件事：**让录入进来的东西能作为结构化资产沉淀与追踪**
（bug 复现步骤、测试用例、迭代/模块归类），而不只是喂给流水线就消失。

> 一句话主线：**人只负责"吐"，工厂负责"炼"与"管"。** 六通道降格为捕获实现细节，
> 用户脑中不再有"我该用哪个通道"的决策——那个决策本身就是摩擦源。

### 0.1 定调判据（防跑偏成 PM 工具）

本系统是**软件工厂**（自主吞吐：需求进→代码出，人只在 2 个审核点出现），
**不是** AI 项目管理工具（禅道/Jira：人管理人的劳动，工件是给人管理的目的）。
"禅道级 bug/测试管理"这个出发点容易把系统带偏，故立硬判据，每个特性都要过：

> **它是在给机器更好的原料 / 让机器自检产出（工厂✓），还是在给人一个管理工作的台子（PM✗）？**

据此明确**不做**：人工测试用例库页面、人维护的模块树、迭代/Sprint 规划、状态看板、
工时、指派、发布版本——这些都是"人管理工作的台子"。**做**：更高质量的需求载体、
AI 生成并供机器自检的验收标准、喂调度的元数据、CR 级遥测。

---

## 1. 总体架构（四块如何咬合）

```
          ┌──────── 捕获层（零结构，多形态）────────┐
语音速录 ──┤ 全局速录 Inbox ── 会话沉淀 ── 现有6通道 ├──┐
          └──────────────────────────────────────────┘  │
                                                          ▼
                            ┌─ gateway::receive(mode) ─────────────┐
                            │  去重 + 安全检查 + 入库               │
                            │  mode=triage → 待整理池（不自动分析） │
                            │  mode=flow   → 直接 enqueue analysis  │
                            └───────────────┬──────────────────────┘
                                            │
        ┌─ 工厂自喂料（调度器）─┐           ▼
        │ 周期扫描 + 提议 Agent ├──► 待整理池 ──(triage Agent 补全/合并/分类)──► 流水线
        └───────────────────────┘                                              │
                                                                                ▼
                            PM 资产层：issues(扩展) + bug_details + test_cases + test_runs
                                       + modules + iterations   （可查询/追踪/看板）
```

核心改动只有一处闸门：**给 `gateway::receive` 增加 `intake_mode`**（`flow` 默认 /
`triage` 待整理）。其余全部围绕它扩展，不破坏现有流水线。

---

## 2. 阶段 A — 语音速录（ASR）⭐ 先落地，体感最强

最小、最独立、风险可控，先做这块拿到"丝滑"的即时反馈。

### 2.1 ASR 配置归属决策（你问的点）

**推荐：Settings 新增独立 section「语音录入」（mic 图标），放在"集成与通知"分组。**

理由与取舍：
| 方案 | 评价 |
|------|------|
| ✅ 新增 `asr` 独立 section | ASR 是**人类输入辅助**，不是 Agent 能力。放独立 section 概念最干净，不会让用户误以为"Agent 会调用麦克风" |
| ❌ 塞进「工具 & MCP」 | web_search 在那里，但那一栏语义是"Agent 自主调用的外部工具（agent loop）"。ASR 不进 agent loop，混进去会误导 |
| ⭕ 合进「Webhook 集成」改名"录入与集成" | 可接受的折中，但 webhook 是被动接收、ASR 是主动录入，耦合勉强 |

落地：`SET_ITEMS` 加 `{ id: 'asr', name: '语音录入', ic: 'mic' }`（需在 `Icon.tsx`
确认/新增 `mic` 图标），渲染 `<AsrSettings />`。

### 2.2 配置存储（照抄 web_search，无需迁移）

存 `app_settings` KV，key 加密走 `secrets`：
```
asr.provider     openai | groq | siliconflow | custom
asr.endpoint     https://api.openai.com/v1/audio/transcriptions
asr.model        whisper-1 | gpt-4o-transcribe | …
asr.api_key      （enc:v1: 密文）
asr.language     zh（可空，自动检测）
```
后端命令（薄包装，逻辑下沉到普通 async fn）：
- `get_asr_settings() -> AsrSettings`（key 只回 `api_key_set: bool`）
- `set_asr_settings(...)`（`secrets::encrypt_field` 写 key）
- `transcribe_audio(audio_base64, mime) -> { text }`

**接口形态**：统一走 **OpenAI 兼容 `/audio/transcriptions`（Whisper multipart）**——
OpenAI / Groq / 硅基流动等都兼容，复用现有"OpenAI 兼容"心智。`core/` 新增
`agents/asr.rs`（纯 Rust，零 Tauri）：multipart POST 音频 → 返回文本。
转写结果视为外部输入，回填前过 `has_obvious_injection()`。

### 2.3 前端交互（话筒 → 悬浮录入框）

1. `Audit.tsx` 的「需求入口」按钮右侧加 `icon-btn` 话筒（`<Icon name="mic" />`），
   `disabled={!activeProject}`。
2. 点击 → 打开**悬浮浮层**（复用 `.proj-select`/`mention-pop` 的浮层定位与 `--win-gutter`
   规范，遮罩 `inset: var(--win-gutter,0)` + `border-radius:14px`，避免圆角窗变方角）。
3. 浮层内：录音按钮（`MediaRecorder` + `getUserMedia`）→ 停止 → base64 传
   `transcribeAudio()` → 文本填入**可编辑 textarea**（允许改错别字）→
   「提交需求」走现有 `submitIssue()`（gateway，mode=flow），或「转待整理」（mode=triage）。
4. 录音中显示电平/计时；尊重 `prefers-reduced-motion`。

### 2.4 风险（必须先验证）

- **WebKitGTK 麦克风权限**：Tauri 2.x Linux webview 下 `getUserMedia` 可能默认被拒，
  需在 `tauri.conf.json` / webview 设置放行媒体权限，或回退到"系统录音→拖文件转写"。
  **这是阶段 A 唯一的硬不确定性，建议第一步就在 `tauri:dev` 里打通麦克风采集。**
- 音频体积：限制单条 ≤ ~25MB / ≤60s，base64 经 IPC；过大改临时文件路径传递。

---

## 3. 阶段 B — 丝滑捕获 Inbox + 捕获/分析解耦

### 3.1 闸门：`intake_mode`

- 迁移 `00NN`：`issues` 增列 `intake_mode TEXT DEFAULT 'flow'`（或复用 `status` 新增
  `triage` 态）+ `raw_capture TEXT`（保存原始未整理文本）。
- `gateway::receive` 签名加 `mode: IntakeMode`；`triage` 时**跳过 `enqueue analysis`**，
  落"待整理池"。现有 6 通道默认仍 `flow`，行为不变。

### 3.2 全局速录（零结构）

- 任意页面一个全局快捷键 / rail 常驻"速录"按钮 → 极简单框，**只有一个文本框**，
  回车即入待整理池（`source_type=quickcapture`, `mode=triage`）。
- 可选绑定当前项目；不选则进"未归属"池，整理时再指派。

### 3.3 Triage Agent（加工自动化）

- 新系统角色 `triage`（走 `forge_role` 绑定 LLM，复用 `run_system_role_text`）：
  把碎片整理为正经 issue（补 title/category/severity、去重合并、建议归属项目/模块）。
- 触发：手动"整理这批"或自喂料调度顺带触发。整理结果进人工审核 1 前的确认。

---

## 4. 阶段 C — 工厂自喂料（修复"扫描没运行"）

### 4.1 周期调度器（照抄 `knowledge.evolve("scheduled")` 范式）

- 在 `lib.rs` setup 里 `tauri::async_runtime::spawn` 一个 interval 循环（**禁止**在业务层
  新增 `tauri::async_runtime` 依赖——调度循环属壳层，符合铁律第 4 条；循环体调用纯 async fn）。
- 读 Settings 的 `autosupply.*` 配置（间隔、开关、每轮上限），到点对所有启用项目跑：
  1. `scan_todos` / `cargo_audit` / `npm_audit`（现成）→ gateway(mode=triage)
  2. **提议 Agent**（新角色 `proposer`）：**工程视角为主**——用 codegraph 指认具体
     符号/文件，**每条工程类提议必须带 file:line 证据**（缺重试、硬编码色值违反
     DESIGN.md、未覆盖的错误分支…），等于"持续版 code-audit-scan"；无证据则丢弃。
     **另允许少量高优先级、强烈建议的新功能提议**（基于路线图/缺口），但需标注
     `kind=feature` 与理由，且每轮数量受限。全部 → 待整理池，**默认只读、只提议**，
     绝不自动进编码。

### 4.2 配置（Settings「并发与流控」或新增「自动供料」卡）

```
autosupply.enabled       bool
autosupply.interval_min   默认 1440（每日）
autosupply.scan_enabled   bool
autosupply.proposer_enabled bool（默认关，需显式开）
autosupply.max_per_run    每轮入池上限，防淹没
```

### 4.3 安全

- 提议 Agent 产出视为外部输入，过 `has_obvious_injection()`。
- 永远落 `triage` 池等人确认，**不得 mode=flow 自动进流水线**，防止工厂自嗨刷需求。

---

## 5. 阶段 D — 需求载体增强 + AI 验收标准（**非** PM 工具）

> 注意定调（§0.1）：不做人工用例库/模块树/迭代/看板。本阶段只做两件**直接喂吞吐**的事：
> 让需求成为机器能更好执行的载体，让机器能自检自己的产出。

### 5.1 Bug 详情 = 需求载体增强（issues 扩展，不另起表）

`issues` 增列（category='Bug' 时启用）：
```
repro_steps TEXT      复现步骤
environment TEXT      环境（OS/版本/分支）
expected    TEXT      期望结果
actual      TEXT      实际结果
```
**定位**：不是给人做 bug 追踪台账，而是把"复现步骤/期望/实际"作为**结构化高质量输入**
喂给 code agent，让工厂能更可靠地**自主修复**。录入时（含语音）category=Bug 展开这组字段；
这些字段同样注入分析/编码上下文。

### 5.2 AI 验收标准（取代"测试用例"，供机器自检）

不建人工用例库。改为：analysis / triage Agent 从 issue **自动生成"验收标准"**
（一组可判定的期望行为），**人只审改不手写**。落 `issues.acceptance_json`（或轻量
`acceptance_criteria` 表，挂 issue）。

**用途是让机器自检产出**，不是给人打勾：
- 注入 code agent 上下文，作为"完成定义（DoD）"约束实现；
- review_2 阶段供审核者快速核对、并可作为自动测试的生成依据。

### 5.3 CR 级测试遥测 `cr_test_runs`

```
id, result TEXT (pass/fail), summary TEXT, related_cr_id, run_at
```
review_2 合并前已自动跑**项目级**测试，把结果落一条记录，作为**工厂遥测**（吞吐质量趋势）。
**只记 CR 整体 pass/fail**，无逐用例映射、无通过率看板。

### 5.4 前端（克制，不新增管理页）

- **不**新增测试用例页 / 迭代页 / 模块树 / 看板。
- 「功能审计」需求详情里：Bug 字段折叠面板 + AI 验收标准只读/可编辑列表（走 `.field`/`.panel`）。
- 列表可加按 category 筛选（自定义下拉），仅为找料方便，非项目管理视图。

---

## 5.9 页面影响图（把控感落地）

### 三层把控 = 现有三页（总览→闸口→显微镜）

| 层 | 页面 | 角色 | 人的动作 |
|---|---|---|---|
| 总览（看） | **Dashboard** | 控制室——看整条传送带流动、各阶段计数、自喂料状态、autonomy 档位 | 观测 |
| 闸口（否） | **Audit** | 逐条裁决 + 录入 + triage 整理；并含**全量需求总账** | 看 / 下钻 / 扔 / 否 |
| 显微镜（究） | **Trace** | 单个 AI 决策的推理过程 | 追问 |

分工铁律：**Dashboard 纯观测（不在此操作单条需求），Audit 才操作**，避免两页都堆需求队列。

### 全量需求总账（已定：落 Audit，seg 切换）

Audit 顶部加 seg：「**待办闸口**」（现状，只显示待我处理）/「**全量总账**」（**新**）。
总账 = 控制室那面**全量玻璃墙**，复用 Audit 现有 列表+详情 架构。

- **看**：每条需求一行，**所有状态**（待整理/分析中/待审1/编码中/待审2/已合并/已拒）；
  强筛选+搜索+分组（项目/状态/分类/来源/时间）；状态列=观测事实(只读)，优先级列=涌现值(点开看为什么，不可拖)。
- **下钻**：点一条 → 详情/裁决，或跳 Trace 看推理。
- **红线（违则退化成 PM）**：不可拖优先级 / 改状态 / 指派 / 规划迭代 / 拉看板泳道。
  判据——逼人做的动作除"看 / 下钻 / 扔 / 否"外多一个都是警报。

### 逐页改动与规模

| 页面 / 组件 | 阶段 | 改动 | 规模 |
|---|---|---|---|
| **Audit.tsx** | A·B·D·把控 | 话筒+录音悬浮层；**全量总账 seg 视图**；triage 池+整理；审核1详情 Bug 字段+AI 验收标准；逐条急停/否决/退回 | 最大 |
| **components/IntakePanel.tsx** | A·B | 语音子能力；手动提交支持 triage；六通道 Tab 弱化为捕获细节 | 大 |
| **Dashboard.tsx** | 把控·B·C | 传送带加第0站"待整理"计数；自喂料状态；autonomy 档位+可调；在途条目深链 Trace | 中大 |
| **Settings.tsx** | A·C·把控 | 「语音录入」section；「自动供料」卡；「门控降级」扩成 autonomy 旋钮 | 中 |
| **App.tsx**（壳/rail） | B | 全局速录入口（rail 按钮→零结构速录框） | 小 |
| **Conversations.tsx** | B | 消息"沉淀为需求"动作（会话即入口） | 小 |
| **Trace.tsx** | 把控 | 控制室深链落点；可加按 proposer/triage 维度筛选 | 小 |
| **Projects.tsx** | C | 手动扫描退居二线（自喂料接管），可加项目级自喂料开关 | 小/可选 |
| **Delivery.tsx** | — | 不涉及（合并后部署/巡检） | 无 |

合计 **6 页 + 1 共享组件**，重头在 Audit + IntakePanel + Dashboard。

---

## 6. 迁移与文件清单（预估）

| 迁移 | 内容 |
|------|------|
| `0037_intake_triage.sql` | issues 加 triage 状态支持 / `raw_capture` / bug 字段 / `acceptance_json` |
| `0038_cr_test_runs.sql` | `cr_test_runs`（CR 级测试遥测，仅此一张，无用例/模块/迭代表） |

| 区域 | 新增/改动 |
|------|-----------|
| 后端 agents | `agents/asr.rs`（纯 Rust 转写）；triage/proposer 提示词进 `roles.rs` |
| 后端 commands | `commands/asr.rs`、`commands/intake.rs`(triage 参数)、验收标准生成挂分析侧 |
| 后端 gateway | `gateway::receive` 加 `mode`；`intake/proposer.rs` |
| 后端 lib.rs | setup 注册自喂料调度 spawn；注册新命令 |
| 前端 services | `transcribeAudio` / `get/setAsrSettings` / 用例&执行 IPC |
| 前端 Settings | `AsrSettings` + `自动供料`卡 |
| 前端 Audit | 话筒按钮 + 录音悬浮层 + triage/筛选 UI |
| 前端 全局 | 速录入口（rail 或快捷键） |
| capabilities | 若 webview 需媒体权限，更新 `main.json` |

---

## 7. 风险与取舍

1. **WebKitGTK 麦克风**（阶段 A）——最大不确定性，必须第一步验证；不通则降级"拖音频文件转写"。
2. **自喂料淹没**——必须 `max_per_run` + 全部进 triage 池 + 提议 Agent 默认关。
3. **退化成 PM 工具**（最大方向性风险）——见 §0.1 判据。一切"给人管理工作的台子"
   （用例库/模块树/迭代/看板/工时/指派）明确**不做**；D 阶段只保留喂吞吐的需求载体增强
   与机器自检用的 AI 验收标准。评审每个新特性都先过这条判据。
4. **issues 表语义过载**——bug 字段加在 issues 上（而非新表）是刻意取舍：复用流水线与
   去重，代价是表变宽；若未来 bug 量极大再拆。

---

## 8. 建议落地顺序

1. **A 语音速录**（独立、体感强、先验证麦克风）→
2. **B 捕获/分析解耦 + 速录 Inbox**（闸门，后续都依赖它）→
3. **C 工厂自喂料**（让供给不枯竭）→
4. **D PM 化**（最大、最后，可再拆子阶段：先 Bug 字段，再测试用例，再迭代/模块）。

每阶段独立可交付、可中止，互不阻塞。
</content>
</invoke>
