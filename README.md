<p align="center">
  <img src="docs/assets/social-preview.png" alt="AutoForge — Human-gated autonomous software factory" width="100%" />
</p>

<h1 align="center">AutoForge</h1>

<p align="center">
  <strong>一条你敢签字放行的自主软件产线。</strong><br />
  让一个人管理一支 AI 工厂，把人的判断力留给真正重要的决策。
</p>

<p align="center">
  <a href="#产品理念">产品理念</a> ·
  <a href="#工作方式">工作方式</a> ·
  <a href="#核心能力">核心能力</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#技术架构">技术架构</a>
</p>

> [!NOTE]
> AutoForge 目前处于 **Alpha** 阶段，适合本地试用、产品验证与内部工作流探索，尚不建议未经评估直接接入关键生产仓库。

## 产品理念

### 软件交付，应该按需求计产，而不是按工时计价

从想法到上线，不该永远被「工程师工时 × 单价」所约束。AI 已经可以承担分析、实现、测试与交付中的大量机械工作；人类稀缺的判断力，应集中在两个问题上：

1. **要不要做？**——需求是否真实、可行、值得投入。
2. **做得对不对？**——代码、测试与风险是否达到合并标准。

AutoForge 不试图再做一个更聪明的聊天助手，而是把 Agent、工具、规则与审计串成一条可运行的软件产线。

| 自主运行 | 人类把关 | 本地优先 |
|:---|:---|:---|
| Agent 承担分析、编码、测试与交付编排 | 需求审核与代码审核是两道明确闸口 | 桌面端、SQLite、进程内调度，无需部署业务服务端 |
| 用流水线消化长尾 backlog | 最终合并权始终留在人手中 | 业务状态保存在本机，模型数据边界由所选 CLI/API 决定 |

## 工作方式

```mermaid
flowchart LR
    A[需求进入] --> B[分析与分流]
    B --> G1{{闸口 1<br/>需求审核}}
    G1 --> C[隔离实现]
    C --> D[测试与安全审计]
    D --> G2{{闸口 2<br/>代码审核}}
    G2 --> E[合并与交付]

    classDef gate fill:#e8772e,color:#16110d,stroke:#f4a93c,stroke-width:2px;
    class G1,G2 gate;
```

- **闸口 1 · 需求审核**：确认问题真实、范围清楚、方案值得执行。
- **闸口 2 · 代码审核**：对照变更摘要、Diff、测试与安全结果决定是否合并。
- **闸口之间 · 自主产线**：Agent 在隔离 worktree 中实现、验证、记录产物，并将结果送回审核队列。

## 核心能力

| 能力域 | AutoForge 提供什么 |
|:---|:---|
| 需求入口 | 快速录入、会议材料、GitHub / Webhook 接入、重复与优先级分析 |
| 蓝图工作室 | 将大型想法迭代为 PRD、规格与可执行任务，再统一送入需求池 |
| 多 Agent 协作 | IM 风格会议室、角色化 Agent、`@mention`、附件与项目上下文 |
| 自主实现 | 在隔离 Git worktree 中执行代码变更，生成变更摘要与交付产物 |
| 质量闸口 | 测试、代码 Diff、功能审计、安全审计与人工批准 |
| 可观测性 | 任务状态、Agent 输出、模型与工具调用链路、失败原因下钻 |
| 知识与扩展 | 项目规格、经验记忆、Skills 与 MCP Server 接入 |

## 为什么可以放手

安全不是产线末端的一项检查，而是整条产线的约束条件。

- **隔离执行**：每个变更在独立 worktree 中完成，避免直接污染主工作区。
- **Git 护栏**：代理层拦截危险操作，合并通过单一受控入口完成。
- **强制验证**：测试与安全审计作为流水线阶段留痕，而不是口头承诺。
- **输入防护**：外部需求、网页与工具返回值按不可信数据处理，检测明显 Prompt Injection。
- **完整追踪**：Agent、模型与工具调用关联到同一条 trace，便于定位结果从何而来。

## 快速开始

### 前提条件

- [Rust](https://rustup.rs/) 1.75+
- Node.js 18+
- 已登录的本地 `claude` CLI，或在应用中配置可用的模型服务
- [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)

### 安装系统依赖

<details>
<summary><strong>Fedora / RHEL</strong></summary>

```bash
sudo dnf install -y dbus-devel gtk3-devel webkit2gtk4.1-devel \
  openssl-devel libappindicator-gtk3-devel librsvg2-devel patchelf
```

</details>

<details>
<summary><strong>Ubuntu / Debian</strong></summary>

```bash
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

</details>

<details>
<summary><strong>macOS</strong></summary>

```bash
xcode-select --install
```

</details>

### 本地运行

```bash
git clone https://github.com/renmengkai/AutoForge.git
cd AutoForge
npm install
npm run tauri:dev
```

仅调试前端界面时，可以运行：

```bash
npm run dev
```

### 常用命令

| 命令 | 用途 |
|:---|:---|
| `npm run dev` | 启动 Vite 前端开发服务器 |
| `npm run build` | 执行 TypeScript 检查并构建前端 |
| `npm run tauri:dev` | 启动完整桌面应用开发环境 |
| `npm run tauri:build` | 构建桌面安装包 |

## 技术架构

| 层次 | 技术 |
|:---|:---|
| 桌面壳 | Tauri 2 |
| 前端 | React 18 · TypeScript · Vite 6 |
| 后端 | Rust · Tokio |
| 数据 | SQLite · 本地文件系统 |
| Agent | 本地 Claude CLI · 可配置 LLM · MCP · Skills |
| 工程隔离 | Git worktree · 受控合并入口 |

```text
src/                 React 页面、组件与设计系统
src-tauri/src/       Rust 运行时、Agent、任务与命令
src-tauri/migrations SQLite 数据模型演进
specs/               Agent 与项目规格
docs/                文档与品牌资产
```

---

<p align="center">
  <strong>AutoForge</strong> · Autonomous where it should be. Human-gated where it matters.
</p>
