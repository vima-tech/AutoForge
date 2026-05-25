# CLAUDE.md — AutoForge 开发指南

## 项目一句话

AutoForge 是一个"Human-Lite-in-the-Loop"自主软件工厂。AI 全自动处理需求发现→分析→实现→预览→测试；人类只在两个审核节点做决策。Claude Code 在 git worktree 中执行代码变更，始终不接触主分支。

## 运行命令

```bash
# 启动完整服务栈
docker compose up -d

# 运行单元测试（不需要 Docker）
pytest tests/unit

# 运行集成测试（需要 Docker 服务运行）
pytest tests/integration

# 代码检查
ruff check .
mypy autoforge/

# 数据库迁移
alembic upgrade head
alembic revision --autogenerate -m "描述变更"

# 查看 Celery 任务日志
docker compose logs -f worker

# 手动触发 Celery 任务（调试）
docker compose exec worker celery -A autoforge.tasks.celery_app call autoforge.tasks.analysis.analyze_issue --args='["<issue-id>"]'
```

## 架构约定

### 异步 / 同步边界

- FastAPI 路由函数：全部 `async def`
- SQLAlchemy 操作：全部使用 `AsyncSession`（`from autoforge.database import get_db`）
- Celery 任务函数本身是同步的（`def run_xxx(self, ...):`），内部用 `_run_async()` 调用 async 代码：

```python
def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()
```

不要在 Celery 任务中使用 `asyncio.run()`——它会与已有 event loop 冲突。

### 数据库访问模式

```python
# FastAPI 路由：用 Depends(get_db)
async def my_route(db: AsyncSession = Depends(get_db)):
    result = await db.scalars(select(MyModel).where(...))

# Celery 任务：用 async_session_factory() 手动管理
from autoforge.database import async_session_factory
async def _do_work():
    async with async_session_factory() as db:
        ...
```

### WebSocket 广播

不要直接访问 `websocket_manager` 的内部 connections dict。总是通过 `broadcast()` 方法，它走 Redis Pub/Sub 确保跨进程传递：

```python
from autoforge.websocket.manager import websocket_manager
await websocket_manager.broadcast(project_id=str(project.id), message={
    "type": "review_needed",
    "issue_id": str(issue.id),
})
```

### 并发槽位管理

Celery 执行任务必须先获取槽位，最终必须释放。模式：

```python
acquired = await concurrency_manager.acquire_slot(cr_id)
if not acquired:
    self.retry(countdown=60)
try:
    # ... 执行工作 ...
finally:
    await concurrency_manager.release_slot()
```

## 安全规则（不可绕过）

### Layer 1 — 输入消毒
所有外部来源的需求标题/描述必须经过 `core/security.py` 的 `sanitize_input()` 过滤，由 `services/input_gateway.py` 的 `submit_issue()` 自动处理。不要绕过 `submit_issue()` 直接写数据库。

### Layer 2 — Git 代理
Claude Code 的所有 git 操作必须通过 `core/git.py` 的 `GitProxy`。禁止以下操作（会抛出 `GitSecurityViolation`）：
- `push` 到 `main` / `master`
- `push --force`
- `branch -D`（删除分支）
- `symbolic-ref`、`update-ref`、`remote set-url`、`config --global`

### Layer 3 — 合并审批
`services/worktree_manager.py` 的 `merge_worktree_to_dev()` 只能由 `api/reviews.py` 的 `review_2` 端点在 decision=`approved` 时调用。不要在其他地方触发合并。

## 目录职责速查

| 目录 | 职责 | 不应该包含 |
|------|------|-----------|
| `agents/` | AI 调用逻辑，返回结构化数据 | 数据库操作、HTTP 路由 |
| `api/` | HTTP 路由，参数校验，调用 service/task | 业务逻辑、Agent 调用 |
| `core/` | 基础设施（限流、安全、并发、配置解析） | 业务逻辑 |
| `services/` | 有状态的业务流程（跨多个 model 的操作） | HTTP 相关代码 |
| `tasks/` | Celery 任务壳（async 逻辑在 services/agents 里） | 复杂业务逻辑 |
| `models/` | SQLAlchemy ORM 定义 | 任何方法（只有 columns + relationships） |
| `schemas/` | Pydantic 请求/响应 Schema | 数据库操作 |

## 添加新功能的步骤

### 新 API 端点
1. 在 `schemas/` 定义请求/响应 Pydantic 模型
2. 在 `api/<module>.py` 写路由函数
3. 如果是新模块，在 `api/__init__.py` 注册 router

### 新 Celery 任务
1. 在 `tasks/<module>.py` 写任务（同步函数 + `_run_async` 桥接）
2. 在 `tasks/celery_app.py` 的 `include` 列表加入模块路径
3. 如需定时触发，在 `beat_schedule` 中添加条目

### 新数据模型
1. 在 `models/<name>.py` 定义 SQLAlchemy 模型（继承 `Base`）
2. 在 `models/__init__.py` 导出
3. 运行 `alembic revision --autogenerate` 生成迁移

## 前端约定

- 所有 API 调用走 `axios`（基础 URL 由 vite proxy 配置，直接用 `/api/v1/...`）
- 服务端状态用 `useQuery` / `useMutation`（TanStack Query）；操作后调用 `queryClient.invalidateQueries`
- Toast 反馈：成功用 `toast.success()`，失败用 `toast.error(e.response?.data?.detail || "操作失败")`
- 样式只用 `index.css` 中定义的 CSS 变量（`var(--accent)`、`var(--surface)` 等），不引入额外 CSS 框架
- 按钮类：`btn btn-primary`、`btn btn-ghost`、`btn btn-danger`、`btn btn-success`，可加 `btn-sm`

## 常见陷阱

**PostgreSQL 异步驱动**：`DATABASE_URL` 必须用 `postgresql+asyncpg://` 前缀（`config.py` 自动替换，无需手动处理）。如果直接用 psycopg2（如预览 DB 的 `CREATE DATABASE`），需要设置 `AUTOCOMMIT=True`，因为 `CREATE DATABASE` 不能在事务中执行。

**Redis 数据库分区**：`REDIS_URL` 用 db=0（WebSocket 状态），`CELERY_BROKER_URL` 用 db=1，`CELERY_RESULT_BACKEND` 用 db=2。三者不要混用。

**Docker socket 权限**：`api` 容器挂载了 `/var/run/docker.sock` 来管理预览容器。本地开发时需要确保当前用户有 docker socket 访问权限。

**worktree 路径**：`WORKTREES_BASE` 默认是 `/tmp/autoforge-worktrees`，容器重启后会丢失。生产环境应挂载持久化卷并配置此路径。

**Celery 任务重试**：任务中 `self.retry(countdown=60)` 会重新入队，注意不要在 retry 前忘记释放已获取的槽位（放在 `finally` 块）。
