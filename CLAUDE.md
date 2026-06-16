# CLAUDE.md — AutoForge

## 项目简介

AutoForge 是一个"Human-Lite-in-the-Loop"自主软件工厂，**Tauri 桌面端应用**。
AI 全自动处理需求分析→代码实现→测试；人类只在两个审核节点做决策。

核心流程：需求提交 → AI 分析 → 人工审核 1 → AI 实现（Claude Code worktree）→ 人工审核 2 → 自动合并

---

## 🎨 设计风格锁定（必读 — 不得跑偏）

本项目 UI 的**设计契约**是根目录的 **[`DESIGN.md`](./DESIGN.md)**（遵循 Google Labs `design.md` 规范，含机器可读 token + 设计意图）。

**任何前端页面/组件改动，必须先读 `DESIGN.md` 并严格遵守，禁止偏离既定风格：**

- 主题：温暖深色优先的「熔炉/余烬（Ember）」风格，唯一品牌强调色是 **ember 橙**；绿/蓝/紫/红/琥珀仅作语义状态色，不作装饰。
- 真源：所有颜色、字号、圆角、阴影、间距**只引用 `src/index.css` 的 CSS 变量**（`var(--ember)`、`var(--bg-2)`、`var(--text-body)` 等），**禁止硬编码**十六进制色值或 px 字号——否则会破坏 dark/light × palette 主题切换。
- 字族：Archivo（display）/ Noto Sans SC（正文）/ JetBrains Mono（代码、标签、kicker）。
- 组件：复用 `src/index.css` 已有类（`.btn` / `.chip` / `.panel` / `.stat` / `.field` / `.seg` / `.proj-select`…），不另起平行样式体系。
- 下拉统一用 `proj-select + mention-pop + mention-row`，**禁用原生 `<select>`**。
- 图标统一走 `<Icon name="..." />`。
- 每屏至多一个 `.btn-primary` 主操作；动效保持微妙且尊重 `prefers-reduced-motion`。

> 改 UI 前先对照 `DESIGN.md` 的 "Do's and Don'ts"。新增颜色/字号/组件时，先在 `src/index.css` 加变量/类，再在 `DESIGN.md` 同步登记，保持二者一致。

---

## ⚠️ Tauri 版本锁定（必读）

本项目使用 **Tauri 2.x**，与 Tauri 1.x 有重大 API 不兼容。**所有涉及 Tauri 的代码修改必须基于 2.x API**，不得参考 1.x 文档或示例。

| 包 | 版本 |
|----|------|
| `tauri`（Rust crate） | **2.11.2** |
| `tauri-build` | **2.6.2** |
| `@tauri-apps/api`（JS） | **2.11.0** |
| `@tauri-apps/cli` | **2.11.2** |
| `tauri-plugin-notification` | 2.x |
| `tauri-plugin-shell` | 2.x |

### Tauri 2.x 关键差异（对比 1.x）

**权限系统（Capabilities）**
- 2.x 必须在 `src-tauri/capabilities/*.json` 中显式声明所有 JS→Rust 调用权限
- 格式：`"core:window:allow-close"`、`"core:window:allow-start-dragging"` 等
- 不声明则运行时报 `not allowed` 错误，**不是代码 bug**
- 当前已声明权限见 `src-tauri/capabilities/main.json`

**窗口 API（JS 侧）**
```typescript
// ✅ Tauri 2.x
import { getCurrentWindow } from '@tauri-apps/api/window';
const win = getCurrentWindow();
win.close(); win.minimize(); win.toggleMaximize();
win.startDragging();   // mousedown 事件中调用，替代 data-tauri-drag-region

// ❌ Tauri 1.x（禁止使用）
import { appWindow } from '@tauri-apps/api/window';
```

**IPC / 事件（JS 侧）**
```typescript
// ✅ Tauri 2.x
import { invoke } from '@tauri-apps/api/core';
import { listen }  from '@tauri-apps/api/event';

// ❌ Tauri 1.x（禁止使用）
import { invoke } from '@tauri-apps/api/tauri';
```

**自定义标题栏拖拽**
- **不要**用 `data-tauri-drag-region`（Linux/WebKitGTK 不支持 `-webkit-app-region`）
- **应该**用 `onMouseDown` → `getCurrentWindow().startDragging()`
- 按钮内必须 `e.stopPropagation()` 阻止冒泡

**透明窗口**
- `tauri.conf.json` 同时设置 `"transparent": true` 和 `"backgroundColor": "#00000000"`
- HTML/body/root CSS 也需要 `background: transparent`

---

## 技术栈

| 层次 | 技术 |
|------|------|
| 桌面壳 | Tauri 2.11.2 |
| 前端 | React 18 + TypeScript + Vite |
| 后端 | Rust（async/tokio） |
| 数据库 | SQLite（sqlx，自动迁移，零外部依赖） |
| AI Agent | LLM API（Anthropic/OpenAI 兼容）+ 本地 `claude` CLI |
| 任务队列 | 进程内 Tokio mpsc channel（无 Redis） |

## 运行命令

```bash
npm install          # 安装前端依赖
npm run tauri:dev    # 完整 Tauri 开发模式（必须用这个测试 IPC/窗口等特性）
npm run tauri:build  # 打包发布
```

> **注意**：`npm run dev`（浏览器模式）无法访问 Tauri IPC、窗口控制、文件系统等特性。

### Linux 系统依赖

```bash
# Fedora
sudo dnf install -y dbus-devel gtk3-devel webkit2gtk4.1-devel \
  openssl-devel libappindicator-gtk3-devel librsvg2-devel patchelf

# Ubuntu/Debian
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

---

## 目录结构

```
src/                          # React 前端
  components/
    Icon.tsx                  # SVG 图标组件（所有 UI 图标统一入口）
    Avatar.tsx                # Agent/用户头像
    Block.tsx                 # 消息块渲染（md/code/file/image/artifact/file_written）
    Markdown.tsx              # Markdown 渲染
  pages/
    Dashboard.tsx             # 工厂总览（流水线统计、需求队列、并发槽位）
    Conversations.tsx         # 会议室（群聊/直聊、消息、项目绑定、工作区文件浏览）
    Projects.tsx              # 项目管理（物料库、代码扫描、备份配置）
    Audit.tsx                 # 变更审核（Diff 查看、预览环境、测试结果）
    Settings.tsx              # 设置（LLM 配置、Agent 管理、系统健康、并发控制）
  services/index.ts           # 所有 IPC 调用封装（唯一 Tauri 交互层）
  data/mock.ts                # BlockType 类型定义 + 开发用 mock 数据
  App.tsx                     # 应用壳（nav rail + 路由）
  index.css                   # 完整设计系统 CSS（CSS 变量、组件样式）

src-tauri/                    # Rust 后端
  capabilities/main.json      # Tauri 2.x 权限声明（必须维护）
  src/
    agents/                   # AI 调用层
      llm.rs                  # LLM API 文本生成（run_agent_text）
      local_claude.rs         # claude CLI 调用（worktree 执行）
      analysis.rs             # 需求分析 Agent
      code_agent.rs           # 代码实现 Agent
    commands/                 # Tauri IPC commands（每个文件对应一个功能域）
      projects.rs             # 项目 CRUD
      issues.rs               # 需求管理
      change_requests.rs      # 变更请求、审核、代码 Diff
      conversations.rs        # 会议室（对话、成员、附件）
      orchestration.rs        # AI 编排（Planner + 多步骤 Agent 执行）
      project_context.rs      # 项目只读上下文（文件浏览、上下文 pin）
      workspace.rs            # 项目工作区（.autoforge/docs + specs 读写）
      settings.rs             # LLM 配置、Agent 管理
      materials.rs            # 物料库文件管理
      specs.rs                # 项目规格（project_specs 表的 CRUD + AI 生成）
      intake.rs               # 需求接收（webhook、GitHub sync、批量导入）
      dev_server.rs           # 开发预览服务器管理
      system.rs               # 系统健康、流水线统计、并发控制
      demo.rs                 # URL 打开等演示功能
    core/
      security.rs             # 输入消毒（has_obvious_injection）
      git.rs                  # GitProxy（拦截危险 git 操作）
      concurrency.rs          # 并发槽位管理（Semaphore + 背压）
      event.rs                # Tauri 事件广播（AppEvent 枚举）
    models/                   # SQLite 数据模型（sqlx::FromRow + Serialize）
    tasks/
      runner.rs               # Tokio 任务池主循环
      analysis.rs             # 需求分析任务
      execution.rs            # 代码实现任务（worktree）
      merge.rs                # 合并任务
    db.rs                     # SQLite 连接池 + 自动迁移
    state.rs                  # AppState（db + job_tx + concurrency + dev_servers）
    lib.rs                    # Tauri setup + 所有 command 注册
  migrations/                 # SQLite 迁移 SQL（按序号顺序执行，不可修改已有文件）
  tauri.conf.json             # Tauri 应用配置
```

---

## 架构要点

### 前端 ↔ 后端通信

全部通过 `src/services/index.ts` 封装的 Tauri IPC，不走 HTTP：

```typescript
import { invoke } from '@tauri-apps/api/core';
const projects = await invoke<Project[]>('list_projects');
const issue    = await invoke<Issue>('submit_issue', { payload: { ... } });
```

后端事件推送用 Tauri Event（`autoforge://event`）：
```typescript
import { listen } from '@tauri-apps/api/event';
listen('autoforge://event', (e) => { /* message_received | conversation_task_updated | ... */ });
```

### 后台任务引擎

`tasks/runner.rs` 维护一个 Tokio mpsc channel，任务入队后异步执行：
- `Analysis` → `agents/analysis.rs` 调用 LLM 分析需求
- `Execution` → 创建 git worktree，调用 `claude --permission-mode acceptEdits`
- `Merge` → `git merge --no-ff` 合并到 dev 分支
- 幂等键写入 `job_executions` 表防重复执行

### AI 编排（会议室任务）

`commands/orchestration.rs` 的 `execute_conversation_task` 流程：
1. 加载项目上下文（claude.md / agents.md + pinned 文件 + 工作区现有文件列表）
2. 必要时压缩历史消息（context_compressor agent）
3. Planner Agent 将用户指令转换为执行计划（`ConversationPlan` JSON）
4. 按步骤执行 Agent（`single` 串行 / `parallel` 并发）
5. 解析 agent 输出中的 `<write-file>` 标签并自动写入工作区文件
6. 可选：summarizer agent 综合发言、doc_writer agent 生成文档产物

### 并发槽位

`core/concurrency.rs` 用 `tokio::sync::Semaphore` 控制同时执行的 CR 数量（默认 5），
`Mutex<usize>` 计数 pending_review，到达阈值（默认 20）时暂停新任务。

### Git 安全代理

所有 git 操作经 `core/git.rs::GitProxy`，正则拦截危险命令：
push main/master、push --force、symbolic-ref、config --global 等。
Claude Code 在 worktree 内执行时额外禁止 git 工具：`--disallowedTools "Bash(git *)"`.

---

## 🧭 长期愿景：后端独立化 + MCP（开发约束 — 必读）

AutoForge 的长期方向是把 Rust 后端从 Tauri 桌面壳中**解耦为可独立运行的服务**，并接入 **MCP 协议**（让 Agent 消费外部工具生态，从"工具"升级为"平台"）。当前**不主动重构**，但**后续所有开发必须保持现有架构缝隙，不得加深 Tauri 耦合**，否则会让未来拆分成本指数级上升。

### 现状（为什么现在拆分成本低）

耦合"浅但宽"，且集中在**传输/壳**层面，不碰业务语义：

- ✅ `agents/llm.rs`（LLM 核心）、`db.rs`、`core/concurrency.rs`、`core/git.rs` 是**纯 Rust**，零 Tauri 引用。
- ✅ `state.rs` 的 `AppState` 只装 `db / job_tx / concurrency / dev_servers`，**不含任何 Tauri 类型**；路径全局走 `OnceLock`。
- ✅ `AppEvent`（`core/event.rs`）是可序列化 enum，本身与 Tauri 无关；只有 `emit()` 一句调 `app.emit`。
- ⚠️ 唯一"宽"的耦合：`AppHandle` 被当事件 sink 一路透传进 `runner → tasks/* → orchestration` 的几乎每个函数签名，仅为了调 `event::emit`。
- ⚠️ 业务逻辑多写在 `#[tauri::command]` 函数体内（inline sqlx），尚未下沉到独立 service 层。

### 开发铁律（新增/修改代码时遵守）

1. **业务逻辑不得依赖 Tauri 类型。** `tasks/`、`agents/`、`core/`、`models/`、新增的 service 函数里**禁止**出现 `AppHandle`、`State<'_, _>`、`Window`、`Manager`、`tauri::*`（事件发射除外，见第 2 条）。需要 DB/并发等依赖时，从 `AppState` 的纯字段取或显式传参。
2. **事件发射只走 `event::emit(app, AppEvent)` 这一个出口**，不要在业务代码里直接 `app.emit(...)` 或新增第二种广播方式。这样未来把 `AppHandle` 换成 `trait EventSink` 时只需改一处。新增事件一律加到 `AppEvent` enum，不要传裸 JSON。
3. **`#[tauri::command]` 保持薄包装。** 命令函数体内尽量只做"取 state → 调普通 async fn → 返回"，把实际逻辑写成不带 Tauri 类型的独立 async fn（放对应 `commands/<module>.rs` 或更下层）。**不要**在命令体里堆积大段业务/sqlx 逻辑。
4. **异步任务用 `tokio::spawn`**（或现有 `tasks/runner.rs` 的入队机制），**不要**在业务层新增对 `tauri::async_runtime` 的直接依赖。
5. **路径/全局配置走 `state.rs` 的 `OnceLock` 初始化器**（`init_*_base`），不要从 Tauri `AppHandle`/`PathResolver` 现取，以便非 Tauri 入口也能初始化。
6. **MCP 相关代码放 Rust 后端，不放前端。** MCP（rmcp）跑在 tokio 上，与后端独立化正交——可在当前 Tauri 进程内先落地，但同样遵守第 1–4 条（MCP client/server 不得引用 Tauri 类型）。MCP 工具结果视为**不可信外部输入**，回灌上下文前过 `has_obvious_injection()`；MVP 阶段只允许**只读/无副作用**工具，写类工具默认禁用并走白名单。

> 一句话：**Tauri 是薄壳不是地基**——把它当成"可替换的传输 + 事件 + 运行时适配层"，业务逻辑始终保持对它无感知。每次提交前自检：我新写的非命令代码里出现 `tauri::` 了吗？如果是事件以外的用途，就是在加深耦合，应改为参数传纯依赖。

---

## 会议室系统

### 对话类型

| 类型 | 说明 |
|------|------|
| `direct` | 与单个 Agent 的直聊（自动为每个可见 Agent 创建） |
| `group` | 多 Agent 群聊，可绑定项目 |

### 群聊绑定项目

创建群聊时可绑定一个项目（`conversations.project_id`）。绑定后：
- 每次触发 AI 任务时，自动注入项目上下文（claude.md、agents.md、pinned 文件、工作区文件列表）
- Agent 可读取所有项目文件（只读），写文件仅限 `.autoforge/` 工作区
- 上下文面板显示工作区文件浏览器

### 消息块类型（BlockType）

定义在 `src/data/mock.ts`：

| 类型 | 说明 |
|------|------|
| `md` | Markdown 文本 |
| `code` | 代码块（含语法高亮、存入工作区按钮） |
| `file` | 文件附件 |
| `image` | 图片附件 |
| `artifact` | 结构化产物（PRD/ADR/测试计划等，含"存入 docs/specs"按钮） |
| `quote_ref` | 引用回复 |
| `file_written` | Agent 写文件操作记录（可展开预览内容） |

### Agent 写文件语法

在项目绑定群聊中，Agent 可在回复里使用以下语法写入工作区：

```
<write-file path=".autoforge/docs/prd.md">
# 产品需求文档
...内容...
</write-file>
```

编排层（`orchestration.rs`）自动解析、写盘，并在消息中插入 `file_written` 块。

---

## .autoforge 工作区规范

每个项目的仓库根目录下有 `.autoforge/` 文件夹，用于存放 AI 对话产物和项目规范。
这是群聊中 **唯一允许写入的目录**，其他项目文件只读。

```
<repo_path>/
  .autoforge/
    docs/       # 产品文档物料库
                # PRD、功能说明、会议记录、决策文档、ADR…
                # Agent 可直接写入；Artifact 块可一键"存入 docs"
    specs/      # 技术规范库
                # 接口定义、架构说明、技术方案、测试计划…
                # Agent 可直接写入；Artifact 块可一键"存入 specs"
    claude.md   # （可选）项目级别的 AI 指引，自动注入群聊上下文
    agents.md   # （可选）项目专属 Agent 角色说明，自动注入群聊上下文
```

### 设计原则

- **docs/** 面向产品/业务：PRD、需求文档、设计决策、会议产物
- **specs/** 面向技术：接口规范、架构文档、实现方案、测试策略
- **claude.md / agents.md**：每次 AI 任务自动注入，无需手动引用
- 其他项目文件（代码、配置等）只读注入上下文，不可修改

### 文件大小限制

- 单文件读取上限：2 MB
- 工作区上下文完整内嵌：≤ 8 KB；超出只列文件名提示

---

## 数据库模型（关键表）

| 表 | 说明 |
|----|------|
| `projects` | 项目（name, slug, repo_path, branch_dev, branch_main） |
| `issues` | 需求条目（project_id, title, status, severity, category） |
| `issue_analyses` | 需求分析结果（分数、分类建议、重复检测） |
| `change_requests` | 变更请求（project_id, issue_id, status, admin_decisions） |
| `worktree_sessions` | Claude Code 执行会话（worktree_path, branch_name） |
| `conversations` | 对话（type: direct/group, project_id, color） |
| `conversation_members` | 对话成员（conversation_id, agent_id） |
| `conversation_project_context` | 群聊 pinned 只读文件路径（rel_path） |
| `messages` | 消息（from_agent, content_json, excluded_from_context） |
| `conversation_attachments` | 消息附件（≤10MB，支持图片/文本/PDF） |
| `conversation_tasks` | AI 任务（planner, plan_json, status） |
| `conversation_task_steps` | 任务步骤（single/parallel，agent_ids_json） |
| `conversation_task_runs` | 单次 Agent 执行记录（output_text） |
| `agents` | Agent 配置（role_type, system_kind, visible_in_chat, mentionable） |
| `llm_configs` | LLM 提供商配置（provider, model, api_key, endpoint） |
| `project_specs` | 项目规格条目（category, title, content，AI 可生成） |
| `material_folders` | 物料库文件夹 |
| `material_files` | 物料库文件 |
| `intake_configs` | 需求接收配置（webhook, GitHub token） |
| `job_executions` | 幂等任务记录（idempotency_key，防重复执行） |

---

## 新功能开发规范

### 新增 Tauri command

1. 在 `src-tauri/src/commands/<module>.rs` 写 `#[tauri::command]` 函数
2. 在 `src-tauri/src/commands/mod.rs` 确认已 `pub mod <module>`
3. 在 `src-tauri/src/lib.rs` 的 `invoke_handler![]` 中注册
4. 在前端 `src/services/index.ts` 添加对应 `ipc<T>(...)` 封装函数
5. **不要**直接在页面组件里调用 `invoke`，统一走 services 层

### 新增数据模型

1. 在 `src-tauri/src/models/<name>.rs` 定义 `#[derive(sqlx::FromRow, Serialize)]` 结构体
2. 在 `src-tauri/migrations/` 添加新的 SQL 文件（`00NN_description.sql`，序号递增）
3. **不可修改已有迁移文件**，只能新增
4. 在 `models/mod.rs` 导出新模型

### 前端页面约定

- 样式只用 `src/index.css` 中的 CSS 变量（`var(--ember)`、`var(--bg-2)` 等）
- 图标用 `<Icon name="..." />` 组件（见 `src/components/Icon.tsx`）
- 自定义下拉使用 `proj-select` + `mention-pop` + `mention-row` 模式（参考 `Audit.tsx`）
- **不使用** `<select>` 原生控件，统一用自定义下拉
- 开发阶段可先用 `src/data/mock.ts` 中的 mock 数据，接入 IPC 后删除

### 消息块扩展

新增 block 类型需同步：
1. `src/data/mock.ts`：在 `BlockType` 联合类型中添加新成员
2. `src/components/Block.tsx`：在 `Block` 组件中添加对应 `if (b.t === ...)` 分支
3. Rust 侧生成该 block 的 JSON（格式必须与 TS 类型一致）

---

## 安全规则（不可绕过）

1. **输入消毒**：所有外部来源的需求经 `core/security::has_obvious_injection()` 过滤
2. **Git 代理**：所有 git 操作经 `GitProxy`，禁止 push main、force push 等危险命令
3. **合并唯一入口**：`review_2` command 的 `approved` 分支是唯一触发 merge 的路径
4. **工作区写入限制**：`workspace.rs` 强制验证写路径必须在 `.autoforge/docs/` 或 `.autoforge/specs/` 内，禁止路径越界（`..` 等）
5. **附件安全**：白名单 MIME 类型，最大 10 MB，存储时 UUID 化文件名
6. **API Key 存储**：所有密钥（LLM `api_key`、MCP `env_json`/`headers_json`、`web_search.api_key`）经 `core/secrets.rs` 信封加密落库——主密钥存系统钥匙环（`keyring` crate，无钥匙环时退化为 app 数据目录下 0600 文件），各密钥用 AES-256-GCM 加密为 `enc:v1:` 密文存 SQLite。写入走 `secrets::encrypt_field`，读取走 `secrets::decrypt`（非密文原样透传，兼容旧明文）；启动时 `migrate_plaintext_secrets` 幂等搬迁残留明文。直接打开 .db 看不到明文密钥

<!-- autoforge:specs:start -->
## AutoForge 项目规格

以下为 AutoForge 管理的项目规格约束，AI 执行任务时必须遵守：

@DESIGN.md
@.autoforge/specs/tech_stack.md
@.autoforge/specs/architecture.md
@.autoforge/specs/coding.md
@.autoforge/specs/api.md
@.autoforge/specs/testing.md
<!-- autoforge:specs:end -->
