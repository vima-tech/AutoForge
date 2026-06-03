# CLAUDE.md — AutoForge

## 项目简介

AutoForge 是一个"Human-Lite-in-the-Loop"自主软件工厂，**Tauri 桌面端应用**。
AI 全自动处理需求分析→代码实现→测试；人类只在两个审核节点做决策。

## 技术栈

| 层次 | 技术 |
|------|------|
| 桌面壳 | Tauri 2.x |
| 前端 | React 18 + TypeScript + Vite |
| 后端 | Rust（async/tokio） |
| 数据库 | SQLite（sqlx，零外部依赖） |
| AI Agent | 本地 `claude` CLI（`claude auth login` 后即可用） |
| 任务队列 | 进程内 Tokio 任务池（无 Redis） |

## 运行命令

```bash
# 安装前端依赖
npm install

# 开发模式（仅前端，在浏览器调试 UI）
npm run dev

# 完整 Tauri 开发模式（需要系统依赖，见下）
npm run tauri:dev

# 打包发布
npm run tauri:build
```

### Linux 系统依赖（Fedora）

```bash
sudo dnf install -y dbus-devel gtk3-devel webkit2gtk4.1-devel \
  openssl-devel libappindicator-gtk3-devel librsvg2-devel patchelf
```

### Linux 系统依赖（Ubuntu/Debian）

```bash
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

## 目录结构

```
src/                     # React 前端
  components/            # Icon, Avatar, Block, Markdown
  pages/                 # Dashboard, Conversations, Audit, Settings
  data/mock.ts           # 开发用 mock 数据（逐步替换为 IPC 调用）
  App.tsx                # 应用壳（nav rail + 路由）
  index.css              # 完整设计系统 CSS

src-tauri/               # Rust 后端
  src/
    agents/              # Claude CLI 调用（local_claude, analysis, code_agent）
    commands/            # Tauri IPC commands（projects, issues, reviews, settings…）
    core/                # 安全、git proxy、并发管理、事件广播
    models/              # SQLite 数据模型（sqlx::FromRow）
    tasks/               # 后台任务（analysis, execution, merge, runner）
    db.rs                # SQLite 连接池 + 自动迁移
    state.rs             # AppState（db + job_tx + concurrency）
    lib.rs               # Tauri setup + command 注册
  migrations/            # SQLite migration SQL 文件
  tauri.conf.json        # Tauri 应用配置
```

## 架构要点

### 前端 ↔ 后端通信

全部通过 Tauri IPC（`invoke`），不走 HTTP：

```typescript
import { invoke } from '@tauri-apps/api/core';

const projects = await invoke<Project[]>('list_projects');
const issue = await invoke<Issue>('submit_issue', { payload: { ... } });
```

后端事件推送用 Tauri Event：

```typescript
import { listen } from '@tauri-apps/api/event';
listen('autoforge://event', (e) => { /* 处理流水线事件 */ });
```

### 后台任务引擎

`tasks/runner.rs` 维护一个 Tokio mpsc channel，任务入队后异步执行：
- `Analysis` → 调用 `claude` CLI 文本模式分析需求
- `Execution` → 创建 git worktree，调用 `claude --permission-mode acceptEdits`
- `Merge` → `git merge --no-ff` 合并到 dev 分支
- 幂等键写入 `job_executions` 表防重

### 并发槽位

`core/concurrency.rs` 用 `tokio::sync::Semaphore` 控制同时执行的 CR 数量（默认 5），
用 `Mutex<usize>` 计数 pending_review，到达阈值（默认 20）时暂停新任务。

### Git 安全代理

所有 git 操作经 `core/git.rs::GitProxy`，正则拦截：
push main/master、push --force、symbolic-ref、config --global 等危险操作。
Claude Code 在 worktree 内执行时额外禁止 `git *` 工具：`--disallowedTools "Bash(git *)"`.

## 新功能开发规范

### 新增 Tauri command
1. 在 `src-tauri/src/commands/<module>.rs` 写 `#[tauri::command]` 函数
2. 在 `src-tauri/src/lib.rs` 的 `invoke_handler![]` 中注册
3. 在前端对应页面用 `invoke('command_name', args)` 调用

### 新增数据模型
1. 在 `src-tauri/src/models/<name>.rs` 定义 `#[derive(sqlx::FromRow, Serialize)]` 结构体
2. 在 `src-tauri/migrations/` 添加新的 SQL migration 文件（`000N_description.sql`）
3. 在 `models/mod.rs` 导出

### 前端页面约定
- 样式只用 `src/index.css` 中的 CSS 变量（`var(--ember)`、`var(--bg-2)` 等）
- 图标用 `<Icon name="..." />` 组件（见 `src/components/Icon.tsx`）
- 开发阶段可先用 `src/data/mock.ts` 中的 mock 数据，接入 IPC 后删除

## 安全规则（不可绕过）

1. **输入消毒**：所有外部来源的需求经 `core/security::has_obvious_injection()` 过滤
2. **Git 代理**：所有 git 操作经 `GitProxy`，禁止危险命令（push main、force push 等）
3. **合并唯一入口**：`review_2` command 的 `approved` 分支是唯一触发 merge 的路径
4. **API Key 存储**：LLM API Key 存入 SQLite，后续迁移到 Tauri keychain plugin
