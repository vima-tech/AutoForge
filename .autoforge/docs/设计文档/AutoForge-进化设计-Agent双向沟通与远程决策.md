# AutoForge 进化设计：Agent 双向沟通与远程决策

> 创建日期: 2026-08-30
> 状态: **设计待评估**（未实施，无代码改动）
> 代号: **ARC** — Ask（问） / Reply（答） / Continue（续）
> 前置分析: [`../待实现功能/AutoForge-异步协作模型-让Agent会开口与远程回执.md`](../待实现功能/AutoForge-异步协作模型-让Agent会开口与远程回执.md)

---

## 0. 文档性质与评估指引

这是一份**可逐条否决的设计规格**，不是方案介绍。评估时请按以下顺序读：

| 想确认什么 | 读哪节 |
|---|---|
| 这事值不值得做 | §1 需求定义 + §2 现状证据 |
| 设计有没有多做 | §1 的 R-ID 追溯表 —— **每个设计条目都标注了它反查哪条需求，反查不到即为未授权复杂度，应当删除** |
| 技术上跑不跑得通 | §4 架构 + §5 关键机制（尤其 §5.2 三计时器） |
| 会不会破坏现有系统 | §9 安全边界 + §5.3 资源治理 + R6 零回归 |
| 要投多少 | §10 里程碑 + §11 风险 |
| 我需要拍板什么 | §12 决策点（5 个，均附建议） |
| 有没有被镀金 | §13 明确不做 |

**本设计遵守的硬约束**（来自全局与项目规范，不可协商）：
- 后端全量 Rust，禁止引入 Node.js / Python 作为后端逻辑载体（`.autoforge/specs/tech_stack.md`）
- 业务逻辑不得依赖 Tauri 类型；事件只走 `event::emit`（`CLAUDE.md` 后端独立化铁律）
- `review_2` approved 是唯一合并入口（`.autoforge/specs/architecture.md`）
- 迁移文件只增不改
- 每处设计能反查到需求，反查不到即删

---

## 1. 需求定义（R-ID 追溯基准）

| R-ID | 需求 | 验证方式 |
|------|------|---------|
| **R1** | 编码 Agent 能在执行过程中就**关键歧义**发问，而不是自行拍板继续 | 构造一个规格未覆盖的歧义任务，观察 agent 发问而非猜测 |
| **R2** | 操作者在手机上能收到该问题，并用**自由文本**回答 | 手机飞书收到问题，回一句话 |
| **R3** | 回答后 agent **带着原上下文继续**，不重跑、不丢 worktree | 同一 `worktree_sessions.id`、同一进程组 pid 全程未变 |
| **R4** | 无人应答时**工厂不卡死**，且不被超时误杀 | 断网/不回答，观察超时后按默认继续、并发槽不被长期占满 |
| **R5** | 该能力**不降低任何现有安全边界** | §9 六条逐条走查 |
| **R6** | 能力关闭时（默认态）行为与今天**完全一致** | 关闭开关后 CLI 命令行 argv 与改造前逐字节相同 |

> **追溯纪律**：下文每个设计条目末尾的 `[R1]` 之类标记即其存在理由。评估者可以据此直接删除任何无标记或标记牵强的条目。

---

## 2. 现状证据（代码级，可复核）

| 事实 | 证据 | 对设计的约束 |
|---|---|---|
| Agent 无法被插话、也无法提问 | `agents/code_agent/cli.rs:122-126`：`stdin.write_all(prompt)` 后立即 `stdin.shutdown()` | 双向通道**不能**走 stdin，必须另辟（→ §4 选 MCP） |
| claude 已跑结构化流 | `cli.rs:455-466`：`--print --output-format stream-json --include-partial-messages --permission-mode acceptEdits` | 出站已实时；且 `tool_use` 可被解析（见下） |
| **已解析 tool_use / tool_result** | `cli.rs:832`（`Some("tool_use")`）、`cli.rs:861`（`tool_result` + `tool_use_id → name` 映射，`cli.rs:186`） | **悬停检测有现成钩子，零新增通道**（→ §5.2） |
| MCP 注入支持 http + headers | `agents/code_agent/mcp_inject.rs:107-121`：http entry 形如 `{"type":"http","url":…,"headers":…}` | **关联 token 可走 header**，无需改协议（→ §5.1） |
| codex 仅支持 stdio MCP；opencode 无逐次注入 | `mcp_inject.rs:8/141`；`CLAUDE.md` 已载明 | 三家能力不齐，必须分档（→ §4.3） |
| **rmcp 只启用了 client feature** | `src-tauri/Cargo.toml:64`：`features = ["client", "transport-child-process", "transport-streamable-http-client-reqwest"]` | 要当 MCP **server** 必须加 feature，属真实改造项（→ §10 M0） |
| 超时会杀掉静默进程 | `cli.rs:200-260`：`wall_deadline`（默认 30min，`system.rs:136`）+ idle（Linux 默认 8min，`system.rs:140-141`，判定为「无输出 **且** CPU 未涨」） | 等人期间两条件全中 → **必被 SIGKILL**，须显式处理（→ §5.2） |
| 并发槽跨整个 job 持有 | `tasks/runner.rs:40-95`：acquire → job 结束才 `slot_released`；默认 5 槽 | 等人会占槽，须设等待配额（→ §5.3） |
| CPU 令牌可租借归还 | `core/cpu_permits.rs:108/133`：`CpuLease` + `acquire(weight)` | 悬停期可归还真实资源（→ §5.3） |
| 现有 feishu 通道是**自定义机器人 webhook** | `core/notify.rs:92-101`：`msg_type=text` + 可选加签，只出不进 | 收消息需**另建**自建应用通道，两者不可混用（→ §7.2） |
| 内置 axum server 默认关闭 | `intake/webhook.rs`（绑 `127.0.0.1`）；`migrations/0013_intake.sql:7` `webhook_enabled DEFAULT 0` | **不能依赖它承载 MCP**，MCP server 需独立生命周期（→ §4.2） |
| 最大迁移序号 | `migrations/0088_retire_build_slots.sql` | 新迁移为 `0089` |

---

## 3. 目标模型：三个新概念

只引入三个概念，不多。

1. **提问点（Question）**——Agent 在执行中对一个具体歧义的发问。它是**带倾向的选择题**，不是开放题：Agent 必须同时给出 `lean`（我倾向哪个）与 `why`（为什么两个都合理），否则视为无效提问。`[R1]`
2. **悬停（Hold）**——Agent 进程仍然活着、上下文完整保留，但暂停计时、归还 CPU、等待答案的一段时间。**悬停不是暂停执行，是暂停计时**。`[R3][R4]`
3. **回执（Reply）**——从外部通道回到工厂的一条答案，携带回答者身份。`[R2][R5]`

> 状态在 Agent 进程内**始终连续**——这是与「杀掉重跑」路线的根本区别，也是 R3 的全部价值所在。

---

## 4. 架构

### 4.1 数据流

```
┌── worktree 内的 claude 进程 ────────────────┐
│  … 写代码 …                                 │
│  遇歧义 → 调 MCP 工具 ask_operator(…)  ─────┼──► [A] 合成 MCP server（进程内, 127.0.0.1:随机端口）
│  （工具调用阻塞，进程保持存活）              │         │ 校验 run_token → 落库 agent_questions
└──────────────▲──────────────────────────────┘         │ status=pending
               │ stdout: tool_use 事件                   ▼
      [B] supervise 解析到 ask_operator      [C] 出站：飞书自建应用发消息（记 channel_msg_id）
          → 进入悬停：暂停 wall/idle 计时                 │
            归还 CpuLease                                 ▼
                                              [D] 手机飞书：人回一句话
                                                          │
      [E] 轮询 im/v1/messages（仅在有 pending 时）◄────────┘
          → 白名单校验 → 按 parent_id 关联 → 落 answer
                       │
      [A] 工具调用返回 answer ──► agent 带上下文继续写 ──► [B] 退出悬停：补偿计时、重借 CpuLease
```

### 4.2 合成 MCP server（不入库、不走用户配置）`[R1][R6]`

- **合成注入**：在 `mcp_inject::build()` 内**额外拼一个内置 server entry**，名为 `autoforge`，**不写 `mcp_servers` 表**。
  理由：`mcp_servers` 是「用户配置的外部 MCP」，把内置能力塞进去会让用户看到一条不该编辑的记录，并需要一条无谓的迁移。`[R6]`
- **生命周期**：仅当 `ask_operator.enabled=1` 时启动，绑 `127.0.0.1:0`（内核分配端口），随 app 生命周期。
  **不复用 intake webhook server**——后者默认关闭（`0013_intake.sql:7`）且面向公开 token，语义与安全面都不同。
- **依赖改动**：`Cargo.toml` 的 `rmcp` 增加 `server` + streamable-http-server transport feature。**这是本设计唯一的依赖改动。**
- **零回归保证**：`enabled=0` 时不启动 server、不合成 entry，`for_claude()` 的入参与今天完全一致 → argv 逐字节相同。`[R6]`

### 4.3 三家 CLI 能力矩阵 `[R1]`

| CLI | MCP 注入 | 悬停检测 | 本设计支持度 |
|---|---|---|---|
| **claude** | http（`mcp_inject.rs:116`） | stream-json `tool_use`（`cli.rs:832`） | **M0 完整支持** |
| **codex** | 仅 stdio（`mcp_inject.rs:141`） | 无结构化流 → 需 MCP 侧共享状态回传 | M2，需 stdio 桥（`current_exe()` 自派子命令，纯 Rust） |
| **opencode** | 无逐次注入入口 | — | **不支持**，行为同今天（与现状一致，非新增缺陷） |

> 分档是能力差异的客观反映，不是设计取巧。M0 只做 claude 一档，**不预先为 codex 抽象接口**（YAGNI，§13）。

---

## 5. 关键机制

### 5.1 关联（correlation）：一次性 run token `[R5]`

MCP server 是进程级共享的，必须知道「这次工具调用属于哪个 CR」。

- 每次 agent run 生成一次性 `run_token`（UUID），注入到 http entry 的 **headers**：`{"X-AutoForge-Run": "<token>"}`（`mcp_inject.rs:118` 的 `headers` 字段已存在，无需改协议）。
- server 侧按 token 反查 `(cr_id, project_id, budget_left)`；token 随 run 结束立即失效。
- **拒绝无 token / 过期 token 的调用**，防止别的本机进程冒用该端口。`[R5]`

### 5.2 悬停：三个计时器的交互 ⚠️ 最易做错处 `[R3][R4]`

现有 `supervise` 循环（`cli.rs:208-260`）里有两个计时器，本设计引入第三个：

| 计时器 | 现值 | 悬停期行为 |
|---|---|---|
| `wall_deadline` | `run_start + 30min`（`system.rs:136`） | **必须补偿**：退出悬停时 `wall_deadline += 悬停时长` |
| `last_activity`（idle，8min，**无输出且 CPU 未涨**才判死） | `system.rs:140-141` | **必须重置**：退出悬停时 `last_activity = now` |
| `answer_deadline`（新） | `ask_operator.wait_timeout_min`，默认 120min | 悬停期唯一生效的计时器；到点按 `default_answer` 返回 |

**为什么只暂停 idle 不够**：等人 40 分钟 → idle 就算豁免，`wall_deadline`（30min，从 run 开始算）照样触发 SIGKILL。**两个都要补偿，缺一必炸。**

**悬停的进入/退出如何检测**（M0，claude）：
- 进入：`handle_claude_line` 解析到 `tool_use` 且 `name == "mcp__autoforge__ask_operator"`（`cli.rs:832` 现成分支，加一个判断）
- 退出：解析到对应 `tool_use_id` 的 `tool_result`（`cli.rs:861` 现成分支，`tool_names` 映射已在 `cli.rs:186`）
- **零新增通道**——这是选择「基于既有解析层」而非「另开 IPC」的理由。

**兜底护栏**（防协议漂移导致悬停不退出）：
- 单次悬停不超过 `answer_deadline`；
- 单次 run 的**累计补偿上限** = `wait_timeout_min × budget_per_cr`，超出即按原墙钟逻辑处死。`[R4]`

### 5.3 资源治理：CPU 令牌 vs 会话槽位 `[R4]`

两类资源，处理方式**故意不同**：

| 资源 | 悬停期处理 | 理由 |
|---|---|---|
| **CPU 令牌**（`cpu_permits.rs:133` `CpuLease`） | **归还**（drop lease），退出悬停时 `acquire` 重借 | 等人期间确实不烧 CPU，占着就是浪费；重借可能排队，但那本就是它该付的代价 |
| **会话槽位**（`concurrency.rs`，默认 5） | **不释放**，但单列 `awaiting_operator` 计数 | 释放后回来可能无槽，会把「等人」变成「等槽」，反而更糟 |

**等待配额（防工厂停摆）**：同时悬停的 CR 数上限 `ask_operator.max_concurrent_holds`（默认 **2**，即 5 槽中最多 2 个在等人）。
达到上限后，新的 `ask_operator` 调用**立即返回默认值**并附注「因等待配额已满，按默认继续」，同时落库标记 `status='budget_denied'` 供事后复盘。`[R4]`

> 这条是 R4 的核心保障：**没有它，5 个 CR 同时等人 = 工厂完全停摆**，且用户在手机上会同时收到 5 个问题而不知先答哪个。

### 5.4 提问质量闸 `[R1]`

Agent 会话痨与 Agent 哑巴一样有害。三道闸：

1. **预算**：`ask_operator.budget_per_cr`，默认 **3**。超出后工具返回「提问预算已用尽，请自行判断并在报告中说明」。
2. **形状强制**：工具 schema 要求 `lean` 与 `why` 必填且非空（§7.1），server 侧校验，空则**拒绝调用并返回纠正提示**——要半成品答案，不要开放题。
3. **prompt 侧约定**：在 `build_prompt` 追加一段「何时该问」（仅当两方案都合理**且**影响面跨模块/不可逆时才问；风格、命名、可局部返工的选择一律自行决定）。

---

## 6. 数据模型（迁移 `0089_agent_questions.sql`）

```sql
CREATE TABLE IF NOT EXISTS agent_questions (
    id                TEXT PRIMARY KEY,
    change_request_id TEXT NOT NULL,
    project_id        TEXT NOT NULL,
    run_token         TEXT NOT NULL,                 -- 关联本次 agent run（§5.1）
    seq               INTEGER NOT NULL DEFAULT 1,    -- 本 CR 内第几问（预算，§5.4）
    question          TEXT NOT NULL,
    options_json      TEXT NOT NULL DEFAULT '[]',
    lean              TEXT NOT NULL DEFAULT '',      -- Agent 的倾向（必填，§5.4）
    why               TEXT NOT NULL DEFAULT '',
    default_answer    TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'pending',
                      -- pending | answered | timeout | budget_denied | abandoned
    answer            TEXT,
    answered_by       TEXT,                          -- feishu:<open_id> | desktop | auto:default
    channel_msg_id    TEXT,                          -- 飞书消息 id（关联回复 + 后续更新卡片）
    asked_at          TEXT NOT NULL DEFAULT (datetime('now')),
    answered_at       TEXT
);
CREATE INDEX IF NOT EXISTS idx_aq_pending ON agent_questions(status, asked_at);
CREATE INDEX IF NOT EXISTS idx_aq_cr      ON agent_questions(change_request_id);
```

**`app_settings` 新增键**（沿用 `execution.timeout_min` 的既有模式，不新建配置表）：

| key | 默认 | 含义 |
|---|---|---|
| `ask_operator.enabled` | `0`（**关**） | 总开关。默认关 = R6 零回归 |
| `ask_operator.budget_per_cr` | `3` | 每 CR 提问上限 |
| `ask_operator.wait_timeout_min` | `120` | 等人超时，到点走默认 |
| `ask_operator.max_concurrent_holds` | `2` | 同时悬停的 CR 上限 |

**不新增**：问题优先级、问题分类、问题模板、回答者组、SLA —— 均无需求反查（§13）。

---

## 7. 接口契约

### 7.1 MCP 工具 schema `[R1]`

```jsonc
{
  "name": "ask_operator",
  "description": "就一个关键歧义征询操作者意见。仅在两个方案都合理、且影响跨模块或不可逆时使用；风格/命名/可局部返工的选择请自行决定。",
  "inputSchema": {
    "type": "object",
    "required": ["question", "lean", "why", "default_answer"],
    "properties": {
      "question":       { "type": "string", "maxLength": 500 },
      "options":        { "type": "array", "items": { "type": "string" }, "maxItems": 4 },
      "lean":           { "type": "string", "description": "你倾向哪个方案（必填）" },
      "why":            { "type": "string", "description": "为什么这是个真歧义、你为何倾向它（必填）" },
      "default_answer": { "type": "string", "description": "无人应答时按此继续（必填）" }
    }
  }
}
```

返回：纯文本答案字符串。三种来源：人工回答 / 超时默认 / 配额拒绝（文本中显式标注来源，让 agent 知道这不是人的判断）。

### 7.2 飞书出站（自建应用）`[R2]`

- 鉴权：`POST /open-apis/auth/v3/tenant_access_token/internal`（`app_id` + `app_secret`）→ token 有效期 2h，**进程内缓存 + 提前 5min 刷新**。
- 发送：`POST /open-apis/im/v1/messages?receive_id_type=chat_id`
  - M0：`msg_type=text`，正文含短码前缀（如 `[Q7]`）+ 问题 + 倾向 + 选项
  - M1：`msg_type=interactive`（卡片 + 按钮）
- 落 `channel_msg_id` 供关联与后续更新。

**通道配置存放**（→ §12 决策点 D3）：复用 `notify_channels`，`kind='feishu_app'`、`target=chat_id`、`secret=` 加密后的 `{"app_id":…,"app_secret":…}` JSON（`core/secrets.rs` 已支持加密任意字符串）。

### 7.3 飞书入站（轮询）`[R2]`

- `GET /open-apis/im/v1/messages?container_id_type=chat&container_id=<chat_id>&start_time=<last_seen>`
- **仅在存在 `status='pending'` 的问题时轮询**，间隔 5s；无待答问题时**完全不轮询** → 常态零开销。这是「轻量」的具体兑现。
- **答案关联**（双路，优先级从高到低）：
  1. 回复消息的 `parent_id == channel_msg_id` —— 用户在飞书里「回复」该条消息，最稳
  2. 正文以短码开头（`[Q7] 用方案A`）—— 兜底
  3. 全场仅一条 pending 且正文无短码 → 归属该条（**可选，默认关**，避免误绑）
- **白名单**：`sender.sender_id.open_id ∈ ask_operator.allowed_open_ids`，不在名单内的回复**忽略且不提示**（不给探测者反馈）。`[R5]`

---

## 8. 状态机

```
                    ┌──────────────► budget_denied（配额满/预算尽，立即返默认）
                    │
  [tool call] ──► pending ──┬──► answered   （白名单内回复，唤醒 agent）
                    │       ├──► timeout    （超 wait_timeout_min，返 default_answer）
                    │       └──► abandoned  （agent 进程已死：墙钟兜底/用户取消/app 退出）
                    │
                    └── 悬停期：wall/idle 暂停，CpuLease 已归还
```

**幂等与恢复**：app 重启后，所有 `pending` 且其 CR 已不在执行态的问题 → 置 `abandoned`（沿用 `lib.rs:143-161` 启动恢复的既有位置与风格——在途任务重排/孤儿回收就在那里）。**问题不重放**——agent 进程已死，答案无处可送。`[R4]`

---

## 9. 安全边界（逐条走查）`[R5]`

| # | 边界 | 措施 |
|---|---|---|
| 1 | **合并铁律不放宽** | M0 **不含**远程 `review_2`。远程审批单列 M3 并单独评估 |
| 2 | **回答者身份** | `open_id` 白名单；`answered_by` 全量落库，与 `admin_decisions.admin_id` 同风格可审计 |
| 3 | **答案是不可信外部输入** | 回灌 agent 前过 `core/security::has_obvious_injection()`（与 MCP 工具结果同规格，`CLAUDE.md` MCP 铁律第 6 条）；长度截断 |
| 4 | **本机端口不被冒用** | 绑 `127.0.0.1:0` + `X-AutoForge-Run` 一次性 token 校验（§5.1） |
| 5 | **凭据加密** | `app_secret` 经 `secrets::encrypt_field` 落库，与 LLM key 同规格（`.autoforge/specs/api.md`） |
| 6 | **不扩大 agent 权限** | `ask_operator` 是**只读征询**，不接受 agent 用它执行任何操作；`--allowedTools` 仅放行 `mcp__autoforge`，不动其余护栏 |

---

## 10. 里程碑与验收

### M0 · 最小闭环（claude + 飞书轮询）—— 本次唯一建议立即启动的

| 改动点 | 文件 | 量级 |
|---|---|---|
| rmcp 加 server feature + 合成 MCP server | `Cargo.toml`、新增 `core/ask_server.rs`（或 `agents/tools/ask_operator.rs`） | 中 |
| 合成注入 + run_token header | `agents/code_agent/mcp_inject.rs` | 小 |
| 悬停检测 + 三计时器补偿 + CpuLease 归还/重借 | `agents/code_agent/cli.rs`（`handle_claude_line` 与 supervise 循环各一处） | **中，风险最高** |
| 等待配额 + 计数 | `core/concurrency.rs` | 小 |
| 迁移 0089 + 模型 | `migrations/`、`models/` | 小 |
| 飞书自建应用通道（出站 + 轮询） | `core/notify.rs` 或新 `core/feishu_app.rs` | 中 |
| Settings 面板（4 个开关 + 通道配置 + 白名单） | `src/pages/Settings.tsx` + `services/index.ts` | 小 |
| 桌面端也能回答（不必开飞书即可自测） | 通知收件箱内联回答入口 | 小 |

**验收（逐条可跑）**：
1. `ask_operator.enabled=0` 时，agent 启动 argv 与改造前**逐字节相同**（diff 两次运行的日志首行）`[R6]`
2. 开启后，构造歧义任务 → 手机飞书收到问题，含 `lean`/`why`/选项 `[R1][R2]`
3. 手机回一句话 → agent **不重跑**继续写完（`worktree_sessions.id` 与进程 pid 全程未变）`[R3]`
4. **不回答**，等待 > 30min（超过原墙钟）→ agent 未被杀，到 120min 按默认继续 `[R3][R4]`
5. 悬停期 `cpu_permits::available()` 上升；退出悬停后回落 `[R4]`
6. 3 个 CR 同时提问，第 3 个收到 `budget_denied` 并按默认继续，工厂未停摆 `[R4]`
7. 非白名单成员在群里回复 → 被忽略，问题仍 pending `[R5]`
8. 回答文本含注入样式串 → 被 `has_obvious_injection` 拦截 `[R5]`
9. 提问 4 次 → 第 4 次收到预算耗尽提示 `[R1]`
10. `cargo test` 全绿 + `tsc` 全绿

### M1 · 卡片交互（长连接）
接飞书长连接订阅 `card.action.trigger`，把「回文本」升级为「点按钮」+ 卡片状态实时翻转。
依赖决策点 D2（自研 WS vs `open-lark` crate）。

### M2 · codex 支持
`current_exe()` 自派 stdio 子命令做 MCP server（纯 Rust，不违反技术栈）；悬停检测改用 MCP 侧共享状态。

### M3 · 远程 review_2（**需单独安全评估，不在本设计授权范围内**）

---

## 11. 风险登记

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| **悬停检测漏判 → agent 被超时误杀** | 中 | 高（任务白跑） | §5.2 双端检测（stream 事件 + MCP 侧登记）互为兜底；累计补偿上限防无限延长 |
| claude stream-json 事件结构变更 | 中 | 中 | 检测失败时**退化为不悬停**（等于今天的行为），不是崩溃；加单测钉住样例行（`cli.rs:1033` 已有此模式） |
| 飞书 API 变更 / 限流 | 低 | 中 | 仅用 3 个最稳定的接口（token/发消息/拉消息）；失败按超时走默认，不阻断流水线（沿用 notify「best-effort 永不阻塞」原则） |
| `open-lark` crate 停止维护 | **高** | 中 | M0 **不依赖它**（纯 reqwest）；M1 再评估，届时可选自研 |
| Agent 滥用提问拖慢流水线 | 中 | 中 | §5.4 三道闸；`agent_questions` 可统计提问率，超标即回调 prompt |
| 用户答非所问 / 答得含糊 | 中 | 低 | 答案原样回灌，agent 可再问一次（在预算内）；不做 NLU 解析（§13） |

---

## 12. 需要拍板的决策点

| # | 决策 | 选项 | 建议 |
|---|---|---|---|
| **D1** | M0 入站用轮询还是长连接 | A 轮询（纯 reqwest）／B 长连接（WS+protobuf） | **A**。零新依赖类别，3–10s 延迟对「等人回消息」完全无感；长连接的价值在卡片按钮，那是 M1 的事 |
| **D2** | M1 长连接怎么实现 | A `open-lark` crate（0.14.0，**近一年未更新**）／B 自研 | **推迟到 M1 再定**，M0 不受影响。届时若 crate 仍无更新，倾向自研（协议面很窄：换端点 + protobuf 帧 + 心跳） |
| **D3** | 飞书应用凭据存哪 | A 复用 `notify_channels`（`secret` 存 JSON）／B 新建表 | **A**。B 多一张表一条迁移，无需求反查；代价是 `secret` 列语义扩展，需在迁移注释写明 |
| **D4** | 悬停期是否释放会话槽位 | A 不释放+配额（本文设计）／B 释放 | **A**。B 会把「等人」变成「等槽」，且引入回来无槽的新失败模式 |
| **D5** | 桌面端要不要也能回答 | A 要（通知收件箱内联）／B 只走飞书 | **A**。开发期自测必需，且不依赖飞书配置即可验证 M0 的 R1/R3/R4，成本很小 |

---

## 13. 明确不做（YAGNI 边界）

以下均**无需求可反查**，本设计**显式拒绝**，未来要做须先立 R-ID：

- ❌ 问题优先级 / 分类 / 标签体系 —— 一次最多 2 个悬停（§5.3），排序无意义
- ❌ 问题模板库、常见问答复用
- ❌ 多人协作：问题指派、抢答、轮值、SLA 计时
- ❌ 答案的 NLU 解析 / 意图识别 —— 原样回灌，让 agent 自己理解
- ❌ 通用「Agent↔人」双向会话框架 —— 目前只有 `ask_operator` **一个**用例，抽象框架属提前抽象（YAGNI 红线②）
- ❌ 把提问能力扩展到分析/评分/测试等其他 Agent —— 需求只说了编码 Agent（红线①：约束别扩面）
- ❌ 微信/钉钉/Slack 的同等入站实现 —— 一个通道打通再说
- ❌ 远程 `review_2` 批准（列为 M3，需单独安全评估）
- ❌ 移动端 Web 页面 —— 属 `DUAL_HEAD.md` Track 2 范畴，与本设计正交

---

## 14. 附：代码证据索引

| 位置 | 内容 |
|---|---|
| `agents/code_agent/cli.rs:122-126` | stdin 喂完即关（哑巴墙根源） |
| `agents/code_agent/cli.rs:186` | `tool_use_id → 工具名` 映射（悬停检测依赖） |
| `agents/code_agent/cli.rs:200-260` | supervise 三计时器循环（wall / idle+CPU 感知） |
| `agents/code_agent/cli.rs:455-466` | claude 启动参数（stream-json / acceptEdits） |
| `agents/code_agent/cli.rs:832 / 861` | `tool_use` / `tool_result` 解析分支（现成钩子） |
| `agents/code_agent/cli.rs:1033` | 既有 stream-json 行解析单测样例（新单测可仿此） |
| `agents/code_agent/mcp_inject.rs:107-121` | http entry 含 `headers`（run_token 载体） |
| `agents/code_agent/mcp_inject.rs:141` | codex 仅 stdio |
| `agents/code_agent/skill_inject.rs` | worktree 注入 + Drop 守卫（如需落文件可仿此） |
| `core/cpu_permits.rs:108/133` | `CpuLease` / `acquire(weight)` |
| `core/notify.rs:92-101` | 现有 feishu = 自定义机器人 webhook（只出不进） |
| `core/security.rs` | `has_obvious_injection()`（答案回灌必经） |
| `commands/system.rs:136/140-141` | 墙钟 30min / idle 8min（Linux）默认值 |
| `tasks/runner.rs:40-95` | 槽位跨 job 持有 |
| `src-tauri/Cargo.toml:64` | rmcp 仅 client feature（需加 server） |
| `migrations/0013_intake.sql:7` | webhook 默认关闭 |
| `migrations/0088_retire_build_slots.sql` | 当前最大迁移序号 |
| `lib.rs:143-161` | 启动恢复：在途任务重排 / `recover_orphaned_reverts` |

### 外部资料

- 飞书官方 · [长连接接收事件](https://feishu.apifox.cn/doc-7518429) / [长连接接收回调](https://feishu.apifox.cn/doc-7518469)（免公网 IP；限企业自建应用；3 秒内响应；每应用 ≤50 连接）
- 飞书官方 · [卡片回传交互回调 card.action.trigger](https://open.feishu.cn/document/feishu-cards/card-callback-communication?lang=zh-CN)
- 第三方 Rust SDK · [open-lark](https://crates.io/crates/open-lark)（0.14.0 / 2025-09-30 / 31,689 下载）
