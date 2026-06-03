# AutoForge

**Human-Lite-in-the-Loop** 自主软件工厂——AI 全自动处理需求分析→代码实现→测试，人类只在两个关键节点审批。

```
需求提交 ──► 分析 Agent ──► 审核 1 ──► Claude Code ──► 审核 2 ──► 合并到 dev
              (自动分类)   (人类批准)   (自动实现)    (人类验证)   (自动测试)
```

## 技术栈

| 层次 | 技术 |
|------|------|
| 桌面壳 | Tauri 2.x |
| 前端 | React 18 + TypeScript + Vite |
| 后端 | Rust + tokio |
| 数据库 | SQLite（零外部依赖） |
| AI Agent | 本地 `claude` CLI |

## 快速开始

### 前提条件

- [Rust](https://rustup.rs/) 1.75+
- Node.js 18+
- `claude` CLI 已登录（`claude auth login`）
- Tauri 系统依赖（见下）

### 安装系统依赖

**Fedora / RHEL：**
```bash
sudo dnf install -y dbus-devel gtk3-devel webkit2gtk4.1-devel \
  openssl-devel libappindicator-gtk3-devel librsvg2-devel patchelf
```

**Ubuntu / Debian：**
```bash
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

**macOS：**
```bash
xcode-select --install
```

### 运行

```bash
# 安装前端依赖
npm install

# 仅前端开发模式（浏览器，无需系统依赖）
npm run dev

# 完整桌面应用开发模式
npm run tauri:dev

# 打包
npm run tauri:build
```

## 项目结构

```
src/              React 前端（页面 + 组件 + 设计系统 CSS）
src-tauri/        Rust 后端（SQLite + Tokio 任务 + Claude CLI）
specs/            Agent 规范文档（分析/编码/测试规范）
docs/             品牌资产
```

## 核心功能

- **需求讨论**：IM 风格的多 Agent 群聊，支持 @mention、代码高亮、文件引用
- **自动分析**：claude-haiku 评估需求真实性、可行性、优先级，检测重复
- **自动实现**：Claude Code 在隔离 git worktree 中执行代码变更，生成报告
- **双重审核**：审核 1（批准分析）→ 审核 2（对比代码 Diff 后批准合并）
- **并发控制**：Tokio Semaphore 控制同时执行数，背压机制防过载
- **安全代理**：GitProxy 拦截危险 git 操作，输入消毒防 Prompt 注入
