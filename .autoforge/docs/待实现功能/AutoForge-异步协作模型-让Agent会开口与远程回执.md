# AutoForge 异步协作模型：让 Agent 会开口、让决定回得来

> 创建日期: 2026-08-30
> 状态: 分析与方案（未实施）
> 触发问题: ①如何脱离「人必须坐在电脑旁」②如何做丝滑的关键点确认沟通，而不是 agent 一路执行到底
> 参考对象: 腾讯 Marvis（马维斯，2026-05 发布的 OS 级 AI 助手）

---

## 0. 一句话结论

AutoForge 缺的不是「更少的人工介入」，而是**把介入点做小、做异步、做可远程**。

现状是两个极端：要么 agent 一路跑到底不吭声（执行期零沟通），要么人必须坐在桌面 GUI 前审一整个 CR 的 diff（审批期重决策）。
中间那一层——**「一句话的选择题」**——今天在 AutoForge 里根本不存在。补上这一层，比做完整的 Web 头更能解决「人被钉在电脑旁」的问题，而且不依赖 DUAL_HEAD 的 312 命令迁移。

---

## 1. 先纠正一个前提：Marvis 不是编程产品

Marvis 是腾讯应用宝团队 2026-05-20 发布的**操作系统层级个人 AI 助手**（Windows/Mac/Android/iOS），
内置 PM、文件管家、系统运维、应用专员、搜索专家、网页交互专家六个 Agent，主打本地文件理解 + 系统操作。
它与 AutoForge 不是同类产品，但它把**「人不在电脑旁」这件事**做成了产品级体验，这部分值得逐条对照。

### 1.1 值得借鉴的四点

| Marvis 机制 | 本质 | 对 AutoForge 的映射 |
|---|---|---|
| **跨端接管**：手机实时看到 PC 执行画面，可随时接管 | 「看得见 + 插得进」是同一件事的两面 | AutoForge 已有实时结构化流（`AppEvent::CodeAgentLog`），但**只能在桌面 GUI 看，且看到了也插不进** |
| **配对极简**：扫码 / 6 位动态配对码 | 远程接入的摩擦要接近零，否则人宁可不用 | AutoForge 已有 `widget_tokens` 的项目级 token 模型（`commands/widget.rs`），是现成的凭据地基 |
| **账号级状态同步**：任务进度/会话历史跨端一致 | 状态跟着人走，不跟着设备走 | 通知收件箱已持久化（`commands/notifications.rs`），差的是「移动端可读的那一屏」 |
| **端云协同双模式**：效率模式 / 本地模式（文件零上传） | 把隐私做成**可切换的档位**而非一刀切 | AutoForge 本就是本地优先；这条印证「远程能力不必以放弃本地隐私为代价」——走内网/中转即可，不必上公网 |

### 1.2 明确**不**该抄的两点

- **桌面画面级远程接管**：Marvis 需要传画面，是因为它操作的是 GUI 应用（Office、浏览器）。AutoForge 的世界是结构化的（CR、diff、测试结果、日志流），传语义比传像素信息密度高一个量级。抄画面接管是南辕北辙。
- **六大 Agent 编排**：AutoForge 的 Planner + 角色卡 + 可插拔编码 Agent 已经比这套更成熟，没有借鉴空间。

---

## 2. 现状诊断：三堵墙（附代码证据）

### 墙一 · 哑巴墙（执行期零沟通）—— 本次的核心

```rust
// src-tauri/src/agents/code_agent/cli.rs:122
if feed_stdin {
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await?; // 关闭 stdin，否则 agent 一直等输入
    }
}
```

prompt 一次性喂完，**stdin 当场关闭**；claude 侧是 `--print --permission-mode acceptEdits`（`cli.rs:455-466`）。
这意味着：

- Agent **无法提问**——问了也没有任何东西在读它的输出并作答。
- 人**无法插话**——想纠偏只能杀掉重跑，丢掉全部上下文。
- 遇到歧义（"用方案 A 还是 B"、"这个字段要不要加索引"）agent 只能**自己拍板往下写**，错了要等到 `review_2` 人看整个 diff 时才发现，返工成本被推到最贵的位置。

这就是「执行到底不沟通」的物理根源。**它不是策略选择，是管道形状决定的。**

### 墙二 · 入口墙（信息出得去，决定回不来）

- **出站**：`core/notify.rs` 已支持 8 类通道（slack / 企业微信 / 飞书 / 钉钉 / ntfy / clawbot 微信 bot / email / webhook），
  在 `review_needed`、`test_failed`、`security_high`、`cr_merged` 等 8 处触发。
- **入站**：`intake/webhook.rs` 有一个真实的 axum server（可配端口 + 项目级 token + 限流），
  但它**只收需求投递**（`/issue`），且 `绑定 127.0.0.1:{port}，仅本机可访问`。
- **审批**：`review_1` / `review_2` 是 `#[tauri::command]`（`commands/change_requests.rs:443/1120`），只能在桌面 GUI 点。

结论：**通知能推到你手机上，你却只能跑回电脑前才能回应它。** 这是最刺眼的不对称，也是最容易补的一环。

### 墙三 · 在线墙（关掉 app = 工厂停摆）

后台任务是进程内 Tokio 队列，桌面进程退出即全停。
`DUAL_HEAD.md` §8 的 Track 2 Gate 第 2 条已经把这件事识别为前置条件（「团队内网是否有一台可常驻在线的机器」）。

---

## 3. 关键洞察：介入点的粒度错了

| | 今天的介入点 | 应该有的介入点 |
|---|---|---|
| 形态 | 一整个 CR 的 diff | 一句话的选择题 |
| 认知成本 | 高（要读代码、要有上下文） | 低（agent 已经给出倾向和理由，人只需点头/换选项） |
| 时机 | 事后（写完了才看） | 事中（写歪之前拦一下） |
| 载体 | 必须桌面 GUI | 手机上一次点击 |
| 后果 | 判错 = 整轮返工 | 判错 = 一步返工 |

Marvis 的「丝滑」感，本质就来自**低成本确认点 + 随时可接管**这两件事同时成立。
AutoForge 现在两件都不成立——不是能力不够，是这条链路从没被铺过。

---

## 4. 方案：四层，按杠杆排序

> 每层都标注了「解决哪堵墙」，以及是否依赖既有路线图，避免与 `DUAL_HEAD.md` 重复造轮子。

### L1 · 让 Agent 会开口 —— 打掉哑巴墙（最高杠杆，不依赖 Web 头）

**机制**：给编码 Agent 注入一个 AutoForge 自己的 MCP 工具 `ask_operator`。

```
ask_operator(
  question: "登录态过期后是静默续签还是跳登录页？",
  options: ["静默续签（我倾向，与现有 refresh 逻辑一致）", "跳登录页"],
  default: 0,          // 超时按此项继续
  why: "两处调用点行为不一致，规格未覆盖"
) -> "静默续签"
```

**为什么这条路可行（三块地基全是现成的）**：

1. **注入通道已存在**：`agents/code_agent/mcp_inject.rs` 已支持给 claude 注入 MCP，且**支持 http transport**（`"type": "http"`，见 `mcp_inject.rs:116`）——AutoForge 可以直接复用进程内的 axum server 暴露这个工具，无需额外进程。
2. **实时流已存在**：claude 已用 `--output-format stream-json --include-partial-messages`（`cli.rs:455-460`），执行画面本就是结构化的。
3. **通知外发已存在**：问题产生后直接走 `core/notify::dispatch` 推到飞书/ntfy/微信。

**流程**：agent 调用 → Rust 侧写一条 `agent_questions` 记录并**阻塞该工具调用** → 推送通知 → 人在手机回答 → 唤醒 → 把答案作为工具结果返回 → **agent 带着完整上下文继续跑，不重跑、不丢 worktree**。

**必须同时处理的四个硬约束**（这是方案成败点，不是细节）：

| 约束 | 现状证据 | 处理方式 |
|---|---|---|
| **超时会杀掉正在等人的 agent** | Linux 默认 idle = 8 分钟（`commands/system.rs:140-141`，非 Linux 默认关闭），判定是「无输出 + CPU 空闲」；墙钟硬上限默认 30 分钟（`system.rs:136`）。等人期间两个条件全中 → **必被 SIGKILL 整个进程组** | 等待期**同时暂停 idle 与墙钟计时**（只暂停 idle 不够，等人超过 30min 照样被墙钟杀），改用独立的「等人超时」（建议默认 2–4h），到点走 `default` 并留痕「未答按默认继续」 |
| **等人期间占着并发槽** | 槽位在整个 job 期间持有（`tasks/runner.rs:60` acquire → job 结束才 `slot_released`），默认只有 5 个 | 等待期**释放 CPU 令牌**（`core/cpu_permits.rs` 已有分相位租约机制，真实资源必须还回去）；会话槽位保留但单列 `awaiting_operator` 计数，并设上限——超过 N 个 CR 同时等人时，后续提问一律走默认，防止工厂被问题堵死 |
| **Agent 会话痨** | — | 每个 CR 提问预算（建议 ≤3 次）；prompt 里硬性要求「必须先给出你的倾向和理由，只在两个方案都合理且影响面大时才问」——**要半成品答案，不要开放题** |
| **三家 CLI 能力不齐** | codex 只能注入 stdio MCP、opencode 无逐次注入入口（`mcp_inject.rs:8/141`，CLAUDE.md 已载明） | claude 走 http MCP（一等公民）；codex 走 stdio bridge；opencode 显式降级为「不提问」，与现状一致，零回归 |

**claude 专属增强（可选）**：`claude --brief` 开启 `SendUserMessage` 工具，让 agent 主动汇报「我准备这么干」；
`--input-format stream-json` 还允许**中途从 stdin 追加用户消息**，这是「人主动插话」而非「agent 提问」的通道——两者可以后续合并成同一条双向管道。

### L2 · 让决定回得来 —— 打掉入口墙（载体选型：飞书自建应用）

**结论：远程回执走飞书「企业自建应用」机器人，优于 ntfy / 自定义机器人 webhook。**

#### 为什么是飞书应用机器人（四条理由，按权重排序）

1. **能回自由文本**——决定性理由。`ask_operator` 的答案常常不是 A/B 选项，而是「用方案 A，但字段名改成 xxx」。
   ntfy 只能点预设按钮，飞书可在会话里直接回一句话。
2. **有会话上下文**——多个问题落在同一话题串，可追问、可翻历史。ntfy 是无状态推送流，问题一多即失序。
3. **免公网**——长连接（WebSocket）模式「无需提供公网 IP 或域名、无需使用内网穿透工具」（官方文档明确），
   本机只出站建连，**不开任何入站端口**，与 AutoForge 本地优先的气质一致。
   ⚠️ 网传「`card.action.trigger` 卡片回调不支持长连接、必须配公网」的说法**是错的或已过时**，官方回调文档给出的正是长连接示例。
4. **IM 级推送保障**——手机端飞书常驻，比 ntfy 不易漏；中文语境自然。

> **旁证**：已有人用飞书长连接 + `card.action.trigger` 给 Claude Code 做**权限审批按钮**（飞书桥接 v0.36.0），
> 场景与本文 L1 几乎同构——这条路被走通过。

#### 三个必须正视的约束

| 约束 | 影响与处理 |
|---|---|
| **仅支持企业自建应用** | 个人版飞书不可用，需注册企业主体（免费，需走流程）。现有 `notify.rs` 的 `feishu` 是**自定义机器人 webhook**（只出不进），与自建应用是两套东西，**不能混用**，需新增通道类型 |
| **回调须 3 秒内响应** | 与 `ask_operator` 的长阻塞天然冲突。设计上正好：回调内**立即 ACK 并更新卡片状态**，答案异步落库后再唤醒被阻塞的 MCP 调用——**绝不能在回调里等人** |
| **Rust 无官方 SDK** | 官方仅 Go/Python/Java/Node。三条路：① 第三方 crate `open-lark`（0.14.0，**2025-09 后近一年未更新**，单人维护，3.1 万下载——可用但需接受依赖风险）；② 自研 WS（protobuf 分帧 + 端点换取 + 心跳重连，成本不低）；③ **禁止**起 Node/Python sidecar 桥接——违反 `.autoforge/specs/tech_stack.md`「后端全量 Rust，不引入 Node.js 服务或 Python 脚本作为后端逻辑载体」 |

#### 推荐落地顺序：先走「出站 + 轮询」，长连接留到第二步

**第一步（真正的轻量档）**：不做事件订阅。自建应用发消息/卡片出去，人在飞书回一句话，
AutoForge 用 `im/v1/messages` API **轮询**该会话取答案。纯 `reqwest`，**零 WebSocket、零 protobuf、零新依赖类别**。
代价仅为轮询延迟（3–10s；人回消息本就慢，无感）与拿不到按钮点击（只解析文本回复）——
而 `ask_operator` 的答案本就更适合自由文本，这个代价几乎不存在。

**第二步（体验升级）**：接长连接订阅 `card.action.trigger`，把「回文本」升级为「点按钮」，
并支持卡片状态实时翻转（已回答/已超时）。

#### 安全铁律（不因远程而放宽）

- **回答者白名单**：按 `open_id` 绑定授权操作者，否则群内任何人都能替你拍板。
- **`review_2` approved 仍是唯一合并入口**：远程回执必须调用同一个 `review_2` 函数，
  记 `admin_id = feishu:<open_id>`（`admin_decisions.admin_id` 字段已存在，`DUAL_HEAD.md` §1.1 已确认为多用户接入点）。
- 若后续仍要开本地 HTTP 回执入口（`intake/webhook.rs` 加 `/reply`），复用现成的 `widget_tokens` 凭据模型
  + `ratelimit.rs` 限流 + `core/rpc.rs` 的 `Principal`/`authorize`（M1 种子已在）。

### L3 · 让状态跟着人走 —— 移动端只读视图（Marvis 的账号同步）

DUAL_HEAD 的 Track 2（Web 头）已规划此事，但它的动机是**团队多人协作**。
针对「单人也要能离开电脑」，只需要它的**最小切片**：一屏只读的流水线状态 + 待答问题列表 + 回答按钮。
不做全功能 Web 头，不等 312 命令迁完。

### L4 · 让工厂常驻 —— 打掉在线墙

headless 二进制 + 常驻机器，属于 `DUAL_HEAD.md` Track 2 M4 范畴，前置条件（`EventSink`、路径注入、无钥匙环密钥回退）大部分已就位。**不建议现在做**——L1+L2 打通后，「人不在电脑旁」的痛点已解决大半，是否值得养一台常驻机器届时会有真实数据支撑。

---

## 5. 与既有路线图的关系

| | DUAL_HEAD Track 2 | 本文 L1+L2 |
|---|---|---|
| 动机 | 团队多人协作 | 单人也要能离开电脑 |
| 前置 | Track 1 完成（312 命令迁移） | **无**，可立即开工 |
| 交付 | Web 头 + 多用户 RBAC | agent 提问链路 + 手机回执 |
| 风险 | 大重构，与新功能开发争抢窗口 | 局部新增，对现有路径零回归（无 `for_code_agent` MCP 时命令与今天完全一致） |

**建议：优先级反过来。** L1/L2 的收益更直接、成本更低、不阻塞任何既有工作。

---

## 6. 建议的第一步（最小可验证切片）

只做一条端到端链路，跑通再扩：

1. `ask_operator` MCP 工具，**只支持 claude**（http transport 注入）。
2. 等待期**同时暂停 idle 与墙钟计时 + 归还 CPU 令牌**（这条不做，功能必然被超时杀掉）。
3. 推送与回收都走**飞书自建应用**：出站发消息，入站用 `im/v1/messages` **轮询**取文本答案（不接长连接、不接卡片按钮）。
4. 回答者按 `open_id` 白名单校验；**不含 review_2 远程批准**（那条涉及合并铁律，单独评估）。
5. 提问预算 ≤3 次/CR，等人超时 2h 走默认值并留痕。

验收：在手机飞书上回一句话回答 agent 的提问，agent 不重跑、带着答案继续写完这个 CR。

---

## 附：本文引用的代码证据

- `src-tauri/src/agents/code_agent/cli.rs:122` — stdin 喂完即关，哑巴墙根源
- `src-tauri/src/agents/code_agent/cli.rs:455-466` — claude `--print` + stream-json + acceptEdits
- `src-tauri/src/agents/code_agent/mcp_inject.rs:107-117` — MCP 注入支持 stdio/http 两种 transport
- `src-tauri/src/agents/code_agent/skill_inject.rs` — worktree 内注入 + Drop 守卫防提交污染（ask_operator 若需落文件可复用此模式）
- `src-tauri/src/intake/webhook.rs:1-60` — 现成 axum server，绑 127.0.0.1，token 鉴权 + 限流
- `src-tauri/src/core/notify.rs` — 8 类出站通道；`dispatch` 触发点见 execution/merge/testing/analysis/revert/security_audit
- `src-tauri/src/commands/system.rs:136/141` — 墙钟默认 30min、空闲默认 8min
- `src-tauri/src/tasks/runner.rs:40-95` — 槽位持有跨整个 job 生命周期
- `src-tauri/src/core/rpc.rs:20-67` — Principal / Role / authorize 已有骨架
- `DUAL_HEAD.md` §1.1 / §8 — 双头地基评估与 Track 1/2 里程碑
- `src-tauri/src/core/notify.rs:92-101` — 现有 `feishu` 通道是**自定义机器人 webhook**（只出不进，支持加签）

## 附二：飞书方案的外部资料

- 飞书官方 · [使用长连接接收事件](https://feishu.apifox.cn/doc-7518429) / [使用长连接接收回调](https://feishu.apifox.cn/doc-7518469)
  ——「无需提供公网 IP 或域名、无需使用内网穿透工具」；限制：仅企业自建应用、3 秒内响应、每应用最多 50 连接
- 飞书官方 · [卡片回传交互回调 card.action.trigger](https://open.feishu.cn/document/feishu-cards/card-callback-communication?lang=zh-CN)
- 第三方 Rust SDK · [open-lark](https://crates.io/crates/open-lark)（0.14.0 / 2025-09-30 / 31,689 次下载，维护活跃度需自行评估）
