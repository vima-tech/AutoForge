# 操作者身份卡 + 通知中心（rail-me 激活）

| 字段 | 值 |
|------|----|
| 状态 | 待实现（提案 v2，2026-06-18 复核仍成立） |
| 优先级 | P1（高 — 填补真实空缺，价值最高） |
| 涉及层 | 前端（rail / Conversations）+ 后端（新命令）+ DB（复用 app_settings） |
| 工作量 | 中（身份卡核心闭环约 0.5–1 天；通知中心持久化版再 0.5–1 天） |
| 相关 | [[AutoForge-自定义LLM图片输入支持-功能提案-v2]]、`core/event.rs`、`commands/settings.rs` |

---

## 1. 背景与问题

导航 rail 底部的「我的账户」头像（`src/App.tsx:338` 的 `<div className="rail-item rail-me">`）
**至今仍是纯静态占位**：

- 无点击处理、无 popover（`src/index.css:950-951` 仅把 hover 背景清空、cursor 设为 default）。
- 头像 `MeAvatar`（`src/components/Avatar.tsx`）**完全硬编码**——固定显示「管」字，无名字、无颜色、无自定义。
- 人类在群聊/直聊里的发言作者写死为「我」（`src/pages/Conversations.tsx:327` `const me = !m.from_agent`、
  `:329` `const author = me ? '我' : ...`；全文检索发言人也写死「我」，见 `:1779`），头像同样是写死的 `MeAvatar`。

也就是说，**操作者（Human-Lite-in-the-Loop 的「人」）在整个系统里没有任何身份**，
而 rail 底部这个 IM 范式里的「黄金位置」被白白浪费。

同时，后端已经在广播一套完整事件流（`core/event.rs` 的 `AppEvent`，**当前 13 个变体**：
IssueCreated / AnalysisCompleted / WorktreeUpdate / **TaskProgress** / PreviewUpdate / TestCompleted /
ReviewNeeded / CrMerged / SecurityAuditCompleted / IterationWarning / PipelineStatus / MessageReceived /
ConversationTaskUpdated），但前端唯一监听器（`src/App.tsx:215`）只用它做两件事：防抖刷新角标/健康、
对 **4 类**事件弹桌面通知（`review_needed` / `iteration_warning` / `cr_merged` / `test_completed`，
见 `App.tsx:227-238`）。**其余 9 类事件落地即丢，没有任何「活动历史 / 收件箱」沉淀。**

## 2. 目标 / 非目标

**目标**
- 让 rail-me 成为「操作者身份配置 + 个人活动收件箱」的统一入口（`mention-pop` 风格 popover）。
- 操作者可设置显示名 / 头像（emoji 或首字母）/ 强调色，并在所有人类发言处生效。
- 把已广播但被丢弃的事件汇聚成可回看的活动流，头像角标显示未读数。

**非目标**
- 不引入登录 / 多用户 / 鉴权概念（本应用是单操作者桌面端）。
- 不替换现有顶栏 dark/light 切换与流水线健康徽标。

## 3. 方案

### 3.1 操作者身份卡

**存储**：复用现有 KV 表 `app_settings`（迁移 `0007_app_settings.sql`，已有
`read_setting` / `write_setting` helper，见 `commands/settings.rs:843-854`），
以 key `operator_profile` 存一段 JSON：`{ display_name, avatar, accent_color, role? }`。
**无需新迁移**。

**命令对**（按 `.autoforge/specs/coding.md`「新增 Command 流程」四步）：
1. `commands/settings.rs` 新增 `get_operator_profile` / `set_operator_profile`（薄包装，逻辑落纯 async fn，遵守 CLAUDE.md 铁律 #3）。
2. `commands/mod.rs` 确认导出。
3. `lib.rs` 的 `invoke_handler![]` 注册。
4. `src/services/index.ts` 加 `getOperatorProfile` / `setOperatorProfile` 封装。
5. `src-tauri/capabilities/main.json` 声明权限。

**接入渲染**：
- `MeAvatar` 改为接受 props（或读全局 store/context），取代写死的「管」与固定背景；
  保留 `--me-avatar-bg` / `--me-avatar-color` 作为默认值。
- `Conversations.tsx:329` 的 `'我'` 与 `:1779` 全文检索发言人改为读 `display_name`（缺省回退「我」）。
- 所有 `MeAvatar` 调用点（`App.tsx:339` 及 Conversations 各处）统一吃 profile。

**（可选，高价值）注入编排上下文**：在 `commands/orchestration.rs` 组装 prompt 上下文时，
带上「操作者显示名」，让 Agent 能正确称呼对话中的人。

### 3.2 通知 / 活动中心

**第一层（零后端改动）**——把 `App.tsx` 唯一监听器收到的 13 类 `AppEvent` 全量分类成活动流：

| 分类 | AppEvent 变体 | 含义 |
|------|--------------|------|
| 🔴 需介入 | `ReviewNeeded`(stage 1/2)、`IterationWarning` | 唯一需人点击 |
| 🟡 进度 | `AnalysisCompleted`、`WorktreeUpdate`、`TaskProgress`、`PreviewUpdate`、`TestCompleted`、`ConversationTaskUpdated` | 流水线推进 |
| 🟢 结果 | `CrMerged`、`SecurityAuditCompleted` | 完成态 |
| 🔵 录入 | `IssueCreated`、`MessageReceived` | 新需求 / 新消息 |

**角标未读数**（拉取式，命令已存在）：`get_badge_counts`（chat_unread + audit_pending）
与 `pipeline_stats`（pending_review_1 + pending_review_2）合成红点数字，复用现成 `.rail-badge`。

**第二层（持久化，推荐）**——事件是瞬时的、刷新即丢，要做真正的收件箱需落库：
- 新增迁移 `00NN_notifications.sql` + `models/notification.rs`。
- 在 `event::emit` 旁的**一个纯 async service fn**（不写进 command 体、不引 Tauri 类型，遵守 CLAUDE.md 铁律 #1/#3）顺手插一行。
- 新增 `list_notifications` / `mark_notification_read` / `mark_all_read` 命令。
- 这样未读状态、跨重启历史、「全部已读」才成立。

> MVP 可先只做「第一层 + sessionStorage 环形缓冲」验证交互形态，再决定是否上持久化表。

## 4. UI 契约（遵守 DESIGN.md）

- popover 一律用 `proj-select + mention-pop + mention-row` 模式，**禁止原生控件**。
- 颜色 / 字号只引用 `src/index.css` CSS 变量；强调色用 `var(--ember)`，语义色仅表达通知分类状态。
- 头像沿用 `.av`（13px 圆角方形 + 右下状态点）规范。
- 角标复用 `.rail-badge`；动效尊重 `prefers-reduced-motion`。

## 5. 验收标准

1. 点击 rail-me 弹出 popover；可编辑名字/头像/色并即时落库（重启后保留）。
2. 群聊与直聊中人类发言的作者名与头像反映 profile 设置。
3. popover 内活动流按 4 类正确分组展示最近事件；点击「需介入」项可跳转对应页面。
4. 有未读时 rail-me 头像显示角标数字，进入活动中心后清零。
5. 持久化版：重启应用后历史通知仍在，已读状态保留。

## 6. 风险与缓解

- **MeAvatar 改 props 影响面**：调用点 + 默认值回退，改动小、可控。
- **事件风暴刷新**：沿用现有 500ms 防抖（`App.tsx` 内），活动流写入同样防抖/批量。
- **持久化表增长**：通知表加保留策略（如仅留最近 N 条 / M 天），避免无限增长。
