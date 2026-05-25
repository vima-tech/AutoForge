# AutoForge

**Human-Lite-in-the-Loop** 自主软件工厂——AI 自动发现需求、生成实现、构建预览，人类仅在两个关键节点审批。

```
外部反馈 ──► 分析 Agent ──► Review 1 ──► Claude Code ──► Review 2 ──► 合并到 dev
(用户/监控/扫描)  (自动分类)  (人类批准)   (自动实现)    (人类验证)   (自动测试)
```

## 功能特性

- **多源需求接入**：网页 Widget、GitHub Issues Webhook、监控告警、管理员手动提交
- **AI 分析**：claude-haiku 自动评估真实性、可行性、优先级，去重检测
- **AI 实现**：Claude Code CLI 在隔离 worktree 中执行代码变更，生成实现报告
- **双重审核**：Review 1（批准分析结果）→ Review 2（对比预览后批准合并）
- **实时预览**：每个变更请求自动启动独立 Docker 预览环境，与生产版本并排对比
- **背压管理**：三阶段（正常 / 降速 / 暂停）防止积压过载
- **自动测试**：合并后自动运行回归测试，发现问题自动转为新需求

## 技术栈

| 层次 | 技术 |
|------|------|
| API | FastAPI + uvicorn |
| 异步任务 | Celery + Redis |
| 数据库 | PostgreSQL (asyncpg) + Alembic |
| AI Agent | Anthropic API (claude-haiku) + Claude Code CLI |
| 预览环境 | Docker SDK + 独立 PostgreSQL 实例 |
| 实时推送 | WebSocket + Redis Pub/Sub |
| 前端 | React + TanStack Query + React Router |

## 快速开始

### 前置要求

- Docker + Docker Compose
- Anthropic API Key
- Claude Code CLI（用于代码执行阶段）

### 1. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env，至少填写：
#   SECRET_KEY=<随机字符串>
#   ANTHROPIC_API_KEY=<你的 API Key>
```

### 2. 启动服务

```bash
docker compose up -d
```

服务启动后：
- Review Portal：http://localhost:3000
- API 文档：http://localhost:8000/docs
- API 健康检查：http://localhost:8000/api/v1/system/health

### 3. 运行数据库迁移

```bash
docker compose exec api alembic upgrade head
```

### 4. 创建第一个项目

```bash
curl -X POST http://localhost:8000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{"name": "My App", "slug": "my-app", "repo_path": "/path/to/repo"}'
```

## 项目结构

```
autoforge/
├── agents/          # AI Agent 实现
│   ├── analysis.py  # 需求分析 Agent（claude-haiku）
│   ├── code_agent.py# Claude Code CLI 调用封装
│   └── test_agent.py# 测试 & 质量检查 Agent
├── api/             # FastAPI 路由
├── core/            # 核心基础设施
│   ├── concurrency.py  # Redis 背压并发管理
│   ├── security.py     # Layer 1 输入消毒
│   ├── git.py          # Layer 2 Git 代理安全审计
│   ├── rate_limit.py   # Redis 滑动窗口限流
│   └── config_parser.py# autoforge.yaml 解析
├── models/          # SQLAlchemy 数据模型
├── schemas/         # Pydantic 请求/响应 Schema
├── services/        # 业务服务层
│   ├── input_gateway.py    # 需求接入网关
│   ├── worktree_manager.py # Git worktree 生命周期
│   ├── preview_orchestrator.py # 预览环境编排
│   └── masking.py          # 预览数据脱敏
├── tasks/           # Celery 异步任务
│   ├── analysis.py  # 需求分析任务
│   ├── execution.py # Claude Code 执行任务
│   ├── preview.py   # 预览环境构建/销毁任务
│   ├── merge.py     # 合并 & 清理任务
│   └── testing.py   # 测试任务（reactive + proactive）
└── websocket/       # WebSocket + Redis Pub/Sub

portal/              # React 审核门户
├── src/pages/       # 页面组件
│   ├── IssueQueuePage.tsx   # 需求队列（首页）
│   ├── Review1Page.tsx      # 审核节点 1
│   ├── Review2Page.tsx      # 审核节点 2
│   ├── PreviewManagementPage.tsx
│   └── SystemStatusPage.tsx
└── src/components/  # 共用组件

widget/              # 用户反馈悬浮窗（纯 JS，无依赖）
specs/               # 各模块规范文档
docs/                # 设计文档
```

## API 端点

### 需求（Issues）

| 方法 | 路径 | 描述 |
|------|------|------|
| `POST` | `/api/v1/issues` | 提交需求（需 `x-admin-id` 或 `x-api-key` 头） |
| `GET` | `/api/v1/issues` | 列出需求（支持 `?status=` 过滤） |
| `GET` | `/api/v1/issues/{id}` | 获取需求详情 |

### 审核（Reviews）

| 方法 | 路径 | 描述 |
|------|------|------|
| `POST` | `/api/v1/reviews/issues/{id}/review-1` | 审核节点 1 决策（approved / rejected） |
| `POST` | `/api/v1/reviews/change-requests/{id}/review-2` | 审核节点 2 决策（approved / revision / rejected） |

### 系统

| 方法 | 路径 | 描述 |
|------|------|------|
| `GET` | `/api/v1/system/status` | 并发状态快照 |
| `GET` | `/api/v1/system/metrics` | KPI 指标（时长、通过率等） |
| `POST` | `/api/v1/system/config` | 热更新并发配置 |
| `GET` | `/api/v1/system/health` | 健康检查 |

### Webhooks

| 方法 | 路径 | 描述 |
|------|------|------|
| `POST` | `/api/v1/webhooks/github` | GitHub Issues 事件（HMAC 验证） |
| `POST` | `/api/v1/webhooks/monitor` | 监控告警接入 |

## 嵌入反馈 Widget

在任意 HTML 页面中加入：

```html
<script
  src="http://localhost:8000/widget/widget.js"
  data-project-id="<project-id>"
  data-api-url="http://localhost:8000"
></script>
```

用户点击悬浮按钮即可提交反馈，直接进入需求队列。

## 项目配置文件（autoforge.yaml）

在被管理的代码仓库根目录放置 `autoforge.yaml`：

```yaml
project:
  name: "My App"
  description: "描述项目背景"

preview:
  docker_image: "myapp:latest"
  port: 8080
  health_path: "/health"
  env:
    NODE_ENV: "preview"

test:
  unit:
    command: "pytest tests/unit"
  integration:
    command: "pytest tests/integration"

quality:
  ruff:
    command: "ruff check ."
  mypy:
    command: "mypy ."

branches:
  main: "main"
  dev: "dev"
  worktree_prefix: "autoforge/"
```

## 安全设计

AutoForge 为 Claude Code 执行实施三层安全防护：

1. **Layer 1 — 输入消毒**（`core/security.py`）：LLM 检测 Prompt Injection，拒绝恶意需求标题/描述
2. **Layer 2 — Git 代理审计**（`core/git.py`）：拦截 `push main`、`push --force`、`branch -D` 等危险操作
3. **Layer 3 — 人工审批合并**（`api/reviews.py`）：worktree 分支必须经管理员在 Review 2 显式批准后才能合并到 dev

Claude Code 在独立 worktree 中执行，无法直接操作主分支。

## 开发

### 安装依赖

```bash
pip install -e ".[dev]"
```

### 运行测试

```bash
pytest tests/unit
pytest tests/integration  # 需要 Docker 环境
```

### 代码检查

```bash
ruff check .
mypy autoforge/
```

### 数据库迁移

```bash
# 生成迁移文件
alembic revision --autogenerate -m "describe change"

# 应用迁移
alembic upgrade head

# 回滚
alembic downgrade -1
```

## 环境变量参考

| 变量 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `SECRET_KEY` | ✓ | — | JWT 签名密钥 |
| `DATABASE_URL` | ✓ | — | PostgreSQL 连接串 |
| `ANTHROPIC_API_KEY` | ✓ | — | Anthropic API Key |
| `PREVIEW_DB_PASSWORD` | ✓ | — | 预览数据库密码 |
| `REDIS_URL` | | `redis://localhost:6379/0` | Redis 连接串 |
| `MAX_CONCURRENT_SLOTS` | | `5` | 最大并发执行槽位 |
| `BACKPRESSURE_PAUSE_THRESHOLD` | | `20` | 待审核积压暂停阈值 |
| `DEBUG` | | `false` | 调试模式 |

## 文档

- [`docs/autoforge-design.md`](docs/autoforge-design.md) — 完整系统设计文档（v0.8）
- [`docs/agent-ui-design.md`](docs/agent-ui-design.md) — Review Portal UI 交互设计文档
- [`specs/`](specs/) — 各模块详细规范（分析/编码/测试/预览/审核）
