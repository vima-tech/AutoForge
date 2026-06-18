# @提及高亮与头像渲染脱离 mock，改用 DB 真源

| 字段 | 值 |
|------|----|
| 状态 | 待实现（提案 v2，2026-06-18 复核仍成立） |
| 优先级 | P2（中 — 用户自建 Agent 体验缺陷 + 违反 CLAUDE.md「接入 IPC 后删 mock」） |
| 涉及层 | 前端 `components/Markdown.tsx`、`components/Avatar.tsx`、`data/mock.ts` |
| 工作量 | 小（约 0.5 天 — 把 mock 查表换成已加载的 DB agents） |
| 相关 | `agents` 表、`Conversations.tsx`、记忆 [[project_agent_two_layer]] |

---

## 1. 背景与问题

CLAUDE.md「前端页面约定」明确：

> 开发阶段可先用 `src/data/mock.ts` 中的 mock 数据，**接入 IPC 后删除**。

但两个**生产渲染组件**至今仍依赖 mock.ts 里那份写死的 5 个 Agent 列表
（`data/mock.ts` 的 `AGENTS` / `AGENT_MAP`，id 固定为 `analyst / coder / tester / architect / security`）：

- **`components/Markdown.tsx:25`**：渲染 Markdown 时用 `AGENTS.find(a => a.name === nm || a.en === nm || nm.startsWith(a.name))`
  检测 @提及并高亮——**只认识这 5 个写死的名字**。用户在设置里新建的真实 Agent，其 @提及不会被识别/高亮。
- **`components/Avatar.tsx:18`**：当传入的是字符串 id 时，用 `AGENT_MAP[agent]` 查头像——
  对不在这 5 个 mock id 内的真实 Agent，**回退查表失败**。

也就是说，**系统的 Agent 早已是 DB 驱动（`agents` 表，两层模型，见 [[project_agent_two_layer]]），
但这两处渲染还停留在 mock 时代**，造成用户自建 Agent 的提及/头像渲染不正确。

> 注：`Block.tsx:5` 与 `Conversations.tsx:20` 也 import mock，但仅取 `BlockType` **类型**（无运行期副作用），不在本提案范围。

## 2. 目标 / 非目标

**目标**
- `Markdown.tsx` 的 @提及检测、`Avatar.tsx` 的 id→Agent 查表，改用运行时从 DB 加载的真实 Agent 列表。
- 移除（或仅保留为类型定义）`mock.ts` 中的 `AGENTS` / `AGENT_MAP` 运行期数据依赖。

**非目标**
- 不改 `BlockType` 等纯类型从 mock.ts 的导入（类型无副作用，可暂留）。
- 不改 Agent 的增删改流程（已完整）。

## 3. 方案

1. 已有 `listAgents()`（或等价）IPC 把真实 Agent 列表带到前端；在 App/Conversations
   层维护一份 `agentMap`（`Conversations.tsx` 已有 `agentMap` 用法，见 `:328`、`:1269`、`:1779`）。
2. 把该 `agentMap` / agent 列表通过 props 或轻量 context 传给 `Markdown` 与 `Avatar`，
   替换对 `AGENTS` / `AGENT_MAP` 的直接 import。
3. @提及检测改为基于真实 Agent 的 `name` / `en` 匹配；找不到时按普通文本渲染（不再误判/漏判）。
4. 清理 `mock.ts`：删除运行期不再被引用的 `AGENTS` / `AGENT_MAP`（确认无其它引用后）。

## 4. 验收标准

1. 在设置里新建一个 Agent 后，群聊里 @其名字能被正确高亮为提及。
2. 该 Agent 的消息头像/小头像正确显示（不再回退失败）。
3. 删除某内置 mock id（如 `architect`）后，已迁移的渲染路径不受影响。
4. `grep -rn "AGENTS\|AGENT_MAP" src/components` 无生产代码运行期引用（仅类型）。

## 5. 风险与缓解

- **加载时序**：Agent 列表异步加载，渲染需容忍「列表未就绪」——未就绪时按纯文本渲染，到位后重渲染即可。
- **性能**：@提及检测在长消息中频繁调用——agentMap 用 Map/对象做 O(1) 查找，避免每次 `Array.find` 全表扫。
