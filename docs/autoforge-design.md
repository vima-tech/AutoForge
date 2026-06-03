# AutoForge — 人类轻度在环的通用软件工厂

**版本：** v0.9
**日期：** 2026-06-03
**状态：** Tauri + SQLite 本地桌面实现阶段

---

## 1. 产品定位

### 1.1 是什么

AutoForge 是一个**项目无关的自进化软件工厂**。

它接入任意 Git 仓库，以用户反馈和自动巡检为需求来源，以 Claude Code 为执行引擎，以实时预览为审核界面，以人类管理员为最终决策者，形成从"发现问题"到"代码上线"的全自动闭环流水线。

工厂内运行的项目是**可实时预览**的——管理员在审核任何改动时，都能打开一个真实运行的预览环境，亲手点击验证效果，而不是盲看 diff 报告。

### 1.4 自进化运行模式

每套运行实例由**目标产品**和**成长框架**两部分组成，二者版本独立：

```
目标产品（持续迭代）       成长框架（固定版本）
─────────────────         ─────────────────
Vocant_latest      +  AutoForge_v1.0   → Vocant 在 v1.0 框架下自进化
AnotherApp_latest  +  AutoForge_v1.0   → AnotherApp 在 v1.0 框架下自进化
AutoForge_latest   +  AutoForge_v1.0   → AutoForge 自身在 v1.0 框架下自进化
```

**AutoForge 自身也是一个目标项目**——它用一个固定版本的成长框架来迭代自己的代码。当新版本的成长框架稳定后，切换到新框架版本即可。这打破了"工厂不能自进化"的悖论：工厂可以自进化，只是进化自己时使用的是固定版本的成长框架，而非正在被修改的那个版本。

成长框架的更新节奏通常快于目标产品，因为工厂本身也在通过运行积累经验、持续改进。

**框架版本切换协议**

从旧框架版本切换到新框架版本时，正在运行的流水线需要妥善处理：

```
切换前提条件：
  · 新框架版本已完成自身的端到端验证（在测试环境中接入过至少一个真实项目）
  · 所有接入项目的管理员收到升级通知，确认切换窗口

切换执行步骤：
  1. 停止接受新需求入队（背压阶段 2 或 3）
  2. 等待所有进行中的 worktree 执行完毕并完成审核，或由管理员手动归档
     （若存在长时间未审核的积压，管理员可选择"带积压切换"，积压条目在新框架下重新执行）
  3. 将成长框架版本号切换为新版本（更新配置）
  4. 验证新框架能正确读取目标项目的 autoforge.yaml 和规范文档
  5. 恢复需求入队，监控前 10 个需求的执行质量

worktree 代码迁移：
  · worktree 分支代码无需迁移，代码已在目标项目的 Git 历史中
  · 未审核的 worktree 会话在新框架下重新创建执行会话，复用原有的 worktree 分支
  · 若规范文档格式在新框架版本中有破坏性变更，需先迁移规范文档再切换框架
```

### 1.2 不是什么

- 不是 CI/CD 工具（Jenkins/GitHub Actions）：AutoForge 负责"想做什么"和"做对了吗"，CI/CD 负责"怎么部署"
- 不是代码审查工具（CodeRabbit）：AutoForge 能自主实现需求，不只是评审
- 不是低代码平台：面向真实代码库，不限技术栈

### 1.3 核心理念

**人类轻度在环（Human-Lite-in-the-Loop）**

```
AI 负责：发现问题、分析可行性、写代码、跑测试、生成报告
人类负责：看预览、拍板决策、附加建议、随时叫停

人类不需要：读 diff、手动测试、写代码、管理分支
```

---

## 2. 接入任意项目

### 2.1 项目配置文件（`autoforge.yaml`）

每个接入 AutoForge 的项目在仓库根目录放置一个配置文件，声明工厂需要知道的一切：

```yaml
# autoforge.yaml — 放在项目根目录

project:
  name: "Vocant"
  description: "AI原生业务管理工具"
  language: python          # python | node | go | ruby | java | ...
  framework: fastapi        # 用于提示 Claude Code 的上下文

# 预览环境配置
preview:
  build:
    command: "pip install -r requirements.txt"
    timeout: 120
  start:
    command: "uvicorn app.main:app --host 0.0.0.0 --port 8000"
    port: 8000
    health_check: "GET /health"
    ready_timeout: 30
  env:
    DATABASE_URL: "${PREVIEW_DB_URL}"   # 工厂注入预览专用数据库
    SECRET_KEY: "${PREVIEW_SECRET_KEY}"
    LLM_API_KEY: "${LLM_API_KEY}"
  seed:
    command: "python seed_data.py"       # 启动后初始化演示数据（可选）
  sensitive_fields:                       # 字段级脱敏声明（M5 实现，见 §5.3）
    - table: users
      fields: [phone, email]
      rule: mask                          # mask | hash | drop
    - table: customers
      fields: [phone, id_card]
      rule: mask

# 测试配置
test:
  unit:
    command: "pytest tests/unit -x --tb=short"
    timeout: 120
  integration:
    command: "pytest tests/integration -x --tb=short"
    timeout: 300
  coverage:
    min_threshold: 0.80       # 覆盖率不得低于此值

# 代码质量检查
quality:
  lint:    "ruff check app/"
  typing:  "mypy app/ --ignore-missing-imports"
  security: "safety check"

# Claude Code 上下文提示
claude_context:
  key_files:
    - "CLAUDE.md"
    - "app/models.py"
    - "app/schemas.py"
  forbidden_paths:
    - ".env"
    - "alembic/versions/"     # 目标项目使用 Alembic 时，迁移文件可新增但不得删除历史
  conventions: |
    - 所有 ID 使用 UUID 字符串
    - 金额使用 Numeric(15,2)
    - 时间戳使用 now_cst()

# 分支策略
branches:
  dev: "dev"
  main: "main"
  worktree_prefix: "autoforge/cr-"

# 反馈入口（可多个）
feedback_sources:
  - type: builtin_widget    # AutoForge 提供的嵌入式反馈组件
  - type: github_issues
    repo: "renmengkai/Vocant"
  - type: webhook
    path: "/webhook/feedback"
```

### 2.2 支持的项目类型

AutoForge 内置适配器，开箱即用：

| 语言/框架 | 预览方式 | 测试框架 |
|----------|---------|---------|
| Python / FastAPI | uvicorn 容器 | pytest |
| Python / Django | gunicorn 容器 | pytest / unittest |
| Node.js / Next.js | npm start 容器 | jest / playwright |
| Node.js / Express | node 容器 | jest / mocha |
| Go | 编译后二进制容器 | go test |
| Ruby / Rails | rails server 容器 | rspec |
| 静态前端 | nginx 容器 | cypress / playwright |

自定义适配器：实现 `Adapter` 接口即可接入任意技术栈。

---

## 3. 核心架构

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           AutoForge Factory                              │
│                                                                          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────────────┐  │
│  │  Input       │    │  Analysis    │    │  Review Portal           │  │
│  │  Gateway     │───→│  Agent       │───→│  （管理员 Web 界面）      │  │
│  │              │    │              │    │  · 需求队列               │  │
│  │ · 用户反馈   │    │ · 真实性评估 │    │  · 审核节点 1 & 2        │  │
│  │ · 管理员输入 │    │ · 可行性分析 │    │  · 实时预览入口          │  │
│  │ · GitHub     │    │ · 优先级建议 │    │  · 管理员建议输入        │  │
│  │ · 巡检发现   │    │ · 分类标签   │    │  · 干预控制面板          │  │
│  └──────────────┘    └──────────────┘    └──────────────────────────┘  │
│                                                        │                 │
│                                                        ↓                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                    Execution Engine                              │   │
│  │                                                                  │   │
│  │  ┌─────────────────────┐    ┌────────────────────────────────┐  │   │
│  │  │  Claude Code Agent  │    │   Preview Orchestrator         │  │   │
│  │  │                     │    │                                │  │   │
│  │  │  · 创建 worktree    │───→│  · 记录 worktree 预览入口     │  │   │
│  │  │  · 读取上下文       │    │  · 管理预览 URL               │  │   │
│  │  │  · 实现代码改动     │    │  · 预留容器化预览生命周期     │  │   │
│  │  │  · 运行测试         │    │  · 预留预览数据库快照         │  │   │
│  │  │  · 生成报告         │    │  · 生命周期管理               │  │   │
│  │  └─────────────────────┘    └────────────────────────────────┘  │   │
│  │                                                                  │   │
│  │  ┌─────────────────────┐    ┌────────────────────────────────┐  │   │
│  │  │   Test Agent        │    │   Notification Hub             │  │   │
│  │  │                     │    │                                │  │   │
│  │  │  · 被动响应（合并后）│    │  · Tauri event 实时推送       │  │   │
│  │  │  · 主动巡检（每日） │    │  · 邮件 / Slack / 企微        │  │   │
│  │  │  · 问题自动入队     │    │  · 预览就绪通知               │  │   │
│  │  └─────────────────────┘    └────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │  Local Runtime Layer                                             │  │
│  │  Tauri 2 · React · Rust commands · SQLite/sqlx · Tokio · Git     │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

当前实现是单机自托管桌面应用：React 管理界面运行在 Tauri WebView 中，前端通过 `invoke()` 调用 Rust `#[tauri::command]`，后端状态由 Tauri `State<AppState>` 持有。SQLite 数据库存放在应用数据目录的 `autoforge.db`，启动时由 `sqlx::migrate!("./migrations")` 自动执行迁移；任务队列使用进程内 `tokio::mpsc`，实时状态通过 Tauri event `autoforge://event` 推送给前端。Docker/Podman、Nginx/Traefik 和外部通知属于预览/通知能力的后续增强，不是当前基础运行依赖。

---

## 4. Claude Code 权限边界（安全隔离）

AutoForge 工厂本身对 Claude Code 是完全只读的。Claude Code 只能在被授权的目标项目路径内操作。

### 4.1 权限矩阵

```
Claude Code 的访问范围
  ├── AutoForge 工厂代码库          ✗ 禁止读写（工厂不会被修改）
  └── 目标项目（worktree 目录内）
        ├── 文件读写                 ✓ 完全权限
        ├── 运行测试 / 构建          ✓ 允许
        ├── 创建新分支（autoforge/*）✓ 允许
        ├── 向 dev 分支提交/合并     ✓ 仅由工厂流程发起，Claude Code 不直接操作
        ├── 修改 main 分支           ✗ 禁止（main 只接受工厂管理员手动合并）
        └── 删除 dev 分支            ✗ 禁止
```

### 4.2 执行沙箱设计

- Claude Code 在独立的 worktree 目录中运行，物理隔离于目标项目的主工作树
- worktree 目录路径白名单由工厂在启动 Agent 前注入，Agent 无法篡改
- Git 操作通过工厂代理层执行，代理层强制拦截已知越权操作（如 `git push origin main`）
- AutoForge 的数据库、配置文件、密钥对 Claude Code 进程不可见

### 4.3 Prompt Injection 防护（三层安全）

Git 代理层拦截已知越权命令，但无法穷举所有绕过路径（`git symbolic-ref`、修改 `.git/config` 远端 URL 等方式几乎无限）。Prompt Injection 是 LLM 驱动系统面临的最严重安全威胁——攻击者可以通过需求入口（Widget、GitHub Issue 等）在正常内容中嵌入恶意指令，覆盖 Claude Code 的安全约束。

因此，权限边界依靠**三层防护**共同保障，而非单一 Git 代理层：

```
┌─────────────────────────────────────────────────────────┐
│  Layer 1：输入消毒层（最早拦截）                          │
│  所有外部输入（Widget/GitHub/API）进入分析 Agent 前，     │
│  先经过独立的 LLM 安全检测，识别并过滤恶意指令内容        │
│  → 恶意内容：直接丢弃，记录安全日志，不进入流水线        │
├─────────────────────────────────────────────────────────┤
│  Layer 2：行为审计层（实时监控）                          │
│  Claude Code 的每一个 Git 操作由独立安全 Agent 实时审计  │
│  → 异常操作（非 autoforge/* 分支写入等）：立即终止会话   │
│  → 操作日志完整保留，供事后审查                          │
├─────────────────────────────────────────────────────────┤
│  Layer 3：分支操作双重确认（最终保障）                    │
│  任何涉及分支合并/删除的指令，必须经管理员手动确认        │
│  → Claude Code 无法独立完成分支操作，只能发起请求        │
└─────────────────────────────────────────────────────────┘
```

**重要**：Layer 1 必须在 M4 之前实现，不能等到 M10 再加。安全层不是功能，是前提条件。

**Layer 1 性能与成本说明**

Layer 1 对每个外部输入额外发起一次 LLM 调用，带来延迟和费用：

| 影响项 | 预估 | 备注 |
|--------|------|------|
| 额外延迟 | 1–3 秒 | 取决于 LLM 响应速度；用户提交反馈后进入异步队列，感知延迟低 |
| 额外 API 成本 | 极低 | 输入消毒的 prompt 短（仅需检测恶意指令），token 消耗远低于分析 Agent |
| 可优化方向 | 本地小模型 | 成本敏感场景可用本地部署的小分类模型替代，牺牲少量精度换取零延迟 |

消毒延迟不影响用户体验——外部输入提交后进入异步队列处理，用户无需等待消毒完成。M4 实现时记录消毒处理的平均延迟作为性能基线。

---

## 5. 实时预览系统（核心设计）

预览系统是 AutoForge 区别于其他工具的核心能力。**人类在审核任何改动时，都能打开一个真实运行的目标系统，亲手操作验证。**

### 5.1 预览环境层级

```
┌─────────────────────────────────────────────────────┐
│  预览层级（同时存在，互相独立）                        │
│                                                      │
│  Production Preview   生产分支（main）的实时镜像      │
│  ─────────────────    供对比参照，始终可访问          │
│  preview.autoforge.io/vocant/main                    │
│                                                      │
│  Dev Preview          dev 分支合并后的当前状态        │
│  ─────────────────    最新通过审核的版本              │
│  preview.autoforge.io/vocant/dev                     │
│                                                      │
│  Worktree Preview     Claude Code 正在修改的版本      │
│  ─────────────────    审核节点 2 的核心审核工具       │
│  preview.autoforge.io/vocant/cr-{id}                 │
│  （worktree 生命周期内有效，合并或丢弃后自动销毁）     │
└─────────────────────────────────────────────────────┘
```

### 5.2 Worktree 预览生命周期

当前 Tauri + SQLite 实现先完成本地 worktree 预览闭环：执行任务创建 Git worktree 后，`preview_environments` 记录 `file://{worktree_path}` 作为预览入口，并通过 Tauri event 通知前端刷新。容器构建、数据库快照注入、热重载和路径路由是 M5 的预览系统增强，不影响主流程状态机。

```
Claude Code 创建 worktree（占用一个并发槽位）
         ↓
Preview Orchestrator 记录 worktree 预览环境
         ↓
当前实现：写入 preview_environments，生成本地 worktree URL
M5 增强：快照预览数据库 → 构建镜像 → 启动容器 → 注入快照数据库
         ↓
健康检查通过 → 预览 URL 就绪
         ↓
推送通知给管理员："预览已就绪，点击查看"
         ↓
Claude Code 持续修改代码
         ↓
当前实现：管理员从 worktree 入口查看结果
M5 增强：文件变更监听 → 热重载（默认，无需重启容器）
         ↓
管理员审核时实时看到最新效果
（管理员可随时点击「重启容器」强制全量重载）
         ↓
审核通过：合并到 dev → dev 预览更新 → worktree 预览终止
          → 快照数据库同步删除（M5）→ 并发槽位释放
审核拒绝：worktree 销毁 → 容器停止（M5）→ 快照数据库删除（M5）→ 槽位释放
```

### 5.3 预览数据要求与数据库策略

目标能力中，每个 worktree 预览使用独立的数据库，避免预览操作污染真实数据。当前实现已在 SQLite 中保留 `db_snapshot_name`、`data_masked_at`、`mask_policy_version` 等字段，实际快照和脱敏执行随 M5 预览系统接入。

**数据质量要求（由规范文档定义，见第 9 章）：**

每个接入项目的 `autoforge.yaml` 必须声明预览数据规范，包含：
- 哪些实体是必须存在的（如：至少 1 个租户、3 个客户、5 个商品）
- 哪些字段必须脱敏（手机、身份证、真实姓名等）
- 数据之间的关联完整性要求
- 适合验证目标需求的特定数据场景（如：库存不足的商品、有冲突的订单）

**快照策略：每个 worktree 创建时独立快照，生命周期与 worktree 完全同步。**

```
预览数据库来源（按优先级）：
1. 从生产数据库做快照（脱敏后）→ 最真实的预览体验，需满足脱敏规范
2. 运行 seed 脚本生成演示数据 → 适合新功能演示，需满足最低数据完整性规范
```

数据库命名规则：`preview_{project}_{cr_id}_{timestamp}`
生命周期：worktree 销毁时**立即删除**，不保留缓存，防止存储空间持续积压。

**基础字段级脱敏规则引擎（M5 实现）**

每个 worktree 预览数据库在对外提供服务前，经过字段级脱敏处理。种子数据中的演示字段（测试手机号、演示姓名等）不以明文形式出现在预览环境。M11+ 的生产快照路径同样复用此引擎。

脱敏规则通过 `autoforge.yaml` 中的 `preview.sensitive_fields` 声明，规则引擎支持三种操作：

| 规则类型 | 操作 | 适用场景 |
|---------|------|---------|
| `mask` | 替换为同类格式占位符（如：`138****8888`、`张*`） | 手机号、姓名、地址等显示类字段 |
| `hash` | 替换为不可逆哈希值 | 需保留唯一性的字段（如：用于内部匹配的 email） |
| `drop` | 清空字段（设为 NULL） | 完全不需要在预览中显示的字段 |

**执行时机**：seed 脚本运行完成后、容器健康检查通过前，规则引擎对快照数据库执行一次全量脱敏 SQL。

**M5 实现范围**：
- 读取 `autoforge.yaml` 中的 `preview.sensitive_fields` 声明
- 对 seed 路径数据库执行字段级替换（按规则类型生成对应 `UPDATE` 语句）
- 记录脱敏执行日志（字段名、处理行数、耗时）供审计

**M11+ 扩展**：生产快照路径接入时，同一规则引擎扩展支持快照数据库的自动脱敏，完成完整脱敏机制。

### 5.4 审核界面中的预览集成

**审核节点 2（实现报告审核）的界面布局：**

```
┌─────────────────────────────────────────────────────────────────┐
│  审核节点 2：实现报告                          [批准] [修改] [拒绝]│
├─────────────────────┬───────────────────────────────────────────┤
│  左：实现报告        │  右：实时预览                              │
│                     │                                           │
│  ## 改动摘要        │  ┌─────────────────────────────────────┐  │
│  为助手页面新增...   │  │  🔴 生产版本                         │  │
│                     │  │  preview.io/vocant/main              │  │
│  ## 修改文件        │  │  ┌─────────────────────────────────┐ │  │
│  · Assistant.jsx    │  │  │  [实际运行的系统截图/iframe]      │ │  │
│  · feedback.py      │  │  └─────────────────────────────────┘ │  │
│                     │  │                                       │  │
│  ## 测试情况        │  │  🟢 本次改动版本                      │  │
│  新增: 3 个         │  │  preview.io/vocant/cr-042            │  │
│  通过: ✓           │  │  ┌─────────────────────────────────┐ │  │
│                     │  │  │  [实际运行的系统 iframe]          │ │  │
│  ## 潜在风险        │  │  └─────────────────────────────────┘ │  │
│  无                 │  │                                       │  │
│                     │  │  [在新标签页打开] [全屏对比]           │  │
│  ## diff 预览       │  └─────────────────────────────────────┘  │
│  + 新增 tag 栏...   │                                           │
│                     │  管理员建议：                             │
│                     │  ┌─────────────────────────────────────┐ │
│                     │  │ 在此输入给 Claude Code 的修改意见... │ │
│                     │  └─────────────────────────────────────┘ │
└─────────────────────┴───────────────────────────────────────────┘
```

### 5.5 预览安全措施

- 预览 URL 需要 AutoForge 登录凭证才能访问（非公开）
- 预览环境与生产环境网络隔离
- 预览数据库中的敏感字段（手机、身份证等）自动脱敏
- 预览环境的写操作不回流到生产数据库
- 每个预览容器有资源上限（CPU/内存）防止单个预览拖垮工厂

---

## 6. 双轨需求入口

### 6.1 外部输入轨（需求驱动）

| 来源 | 接入方式 | 说明 |
|------|---------|------|
| 嵌入式反馈组件 | JS SDK，一行代码嵌入任意前端 | AutoForge 提供，用户在目标系统内直接反馈 |
| 管理员手动提交 | Review Portal 界面 | 新功能需求、已知问题 |
| GitHub Issues | Webhook 自动同步 | 开发者提交的 Issue |
| 监控告警 | Sentry / Grafana Webhook | 错误聚合后自动创建 Issue |
| API 接入 | REST API | 任意外部系统推送需求 |

**嵌入式反馈组件（`autoforge-widget`）：**
```html
<!-- 在任意项目前端加入此脚本，即可获得反馈按钮 -->
<script src="https://cdn.autoforge.io/widget.js"
        data-project-id="vocant-prod"
        data-api-key="af_xxxx">
</script>
```
效果：在页面右下角出现反馈按钮，用户点击后弹出反馈表单，截图当前页面一并上传。

**Widget 数据隐私策略（M10 实现前必须完成设计）：**

Widget 面向终端用户，是系统中最不可控的需求入口——用户可能提交个人信息、公司机密或截图中包含敏感数据。以下策略须在 Widget 对外发布（M10）前完成实现：

| 类别 | 策略 |
|------|------|
| 数据最小化 | 只收集必要字段（内容文本、页面 URL、截图）；不收集用户身份信息 |
| 截图脱敏 | 上传前在客户端对截图中的输入框内容自动模糊处理（可配置关闭） |
| 存储保留期 | 原始反馈数据保留 180 天后自动删除；已转化为 Issue 的条目随 Issue 生命周期保留 |
| 提交者标识 | 只存 IP 哈希（不可逆），不存原始 IP；不存 Cookie 或设备指纹 |
| 数据删除请求 | 提供管理员接口可按 IP 哈希批量删除反馈数据 |
| 敏感内容过滤 | 输入消毒层（Layer 1）同时过滤明显的个人敏感信息（手机号、身份证号等格式） |

> 注：当前 AutoForge 定位为**自托管私有部署**，不涉及跨境数据传输，GDPR/CCPA 合规要求取决于部署方的业务场景，由部署方自行负责。工厂提供上述最低隐私保护机制，不做合规背书。

### 6.2 内部巡检轨（质量驱动）

测试 Agent 以双模式运行：

**模式 A：被动响应**（每次合并到 dev 后触发）
- 验证本次改动的功能正确性
- 执行关联模块回归测试

**模式 B：主动巡检**（每日定时 + 手动触发）
- 全量测试套件
- 代码质量扫描（lint / typing / security）
- 性能基准对比（与上次基准的 delta）
- 依赖安全漏洞检查
- 死代码检测

**巡检发现问题后：**
```
问题严重级别
  ├── Critical → 立即通知 + 自动创建高优先级 Issue
  ├── High     → 创建 Issue + 通知
  ├── Medium   → 创建 Issue，进入常规队列
  └── Low      → 汇入每周质量周报，不单独创建 Issue
```

---

## 7. 完整生命周期

所有来源的需求都经过 Agent 分析 + 人类双重审核后才进入实现环节。入口处的主要防护是限流，防止恶意刷入；需求质量由双重审核保障。

```
 外部输入                              内部巡检
─────────────                         ────────────
嵌入反馈组件                           每日定时巡检
管理员提交             ┌────────────┐  合并后触发
GitHub Issue  ──────→ │  限流网关  │ ←── 发现问题自动入队
监控告警              │（防恶意刷入）│
                      └─────┬──────┘
                            ↓
                 ┌──────────────────────┐
                 │   需求分析 Agent      │
                 │   · 需求真实性分析    │
                 │   · 可行性 / 影响评估 │
                 │   · 分类 / 优先级建议 │
                 │   · 重复检测          │
                 │   · 形成修改方案摘要  │
                 └──────────┬───────────┘
                            ↓
             ┌──────────────▼───────────────┐
             │   《审核节点 1》人类管理员      │
             │   · 查看 Agent 分析报告       │
             │   · 可附加实现建议、约束条件  │
             │   · 只有有价值的需求才通过    │
             └──────┬──────────────┬────────┘
                 拒绝│            批准│
                    ↓               ↓
                  归档    ┌─────────────────────────────┐
                          │  Claude Code Agent           │
                          │  读取规范文档 + 修改方案     │
                          │  创建 worktree               │
                          │            ↓                │
                          │  Preview Orchestrator        │
                          │  按数据规范启动预览容器      │
                          │            ↓                │
                          │  实现代码改动                │
                          │  严格执行初步测试            │
                          │  生成实现报告 + 预览就绪     │
                          └──────────────┬──────────────┘
                                         ↓
                     ┌───────────────────▼──────────────────┐
                     │   《审核节点 2》人类管理员              │
                     │   左：实现报告 + diff                 │
                     │   右：【实时预览】（生产 vs 改动对比）  │
                     │   · 可亲手点击验证功能                 │
                     │   · 可附加修改建议                    │
                     │   ⚠️ 迭代 ≥3 轮：置顶高亮 + 系统建议  │
                     │      "手动介入或重新描述需求"          │
                     └──────┬──────────┬────────────┬───────┘
                          拒绝│        修改│          批准│
                             ↓          ↓              ↓
                        丢弃        追加建议        合并到 dev
                        worktree  → Claude Code    预览升级为
                        预览销毁    新一轮迭代      dev 预览
                                  （软上限 3 轮，
                                   超出后系统提醒
                                   但不强制终止）
                                                      ↓
                                           ┌──────────────────┐
                                           │   测试 Agent      │
                                           │   按测试规范执行  │
                                           │   （被动响应）    │
                                           └──────┬───────┬───┘
                                               通过│    失败│
                                                  ↓       ↓
                                             生命周期   Bug 自动
                                               结束    入队 → 循环
```

### 7.1 背压式流量控制（审核积压管理）

工厂不强求持续运转。当管理员审核速度跟不上 Claude Code 执行速度时，系统通过三阶段渐进降速来适应，而不是强制清理资源。

**"待审核积压数"**：已完成实现、正在等待管理员审核节点 2 的条目数量。

```
┌──────────────────────────────────────────────────────────┐
│  阶段 1：正常模式                                          │
│  待审核积压 < 5（并发上限未全部占满）                      │
│  → 最多 5 个 worktree 并发执行                           │
│  → 系统全速运行                                          │
├──────────────────────────────────────────────────────────┤
│  阶段 2：单线程降速模式                                    │
│  并发槽位全部被"待审核"条目占满（5/5）                    │
│  → 新需求不再并发，切换为单线程执行（1 个并发）           │
│  → 已有的 5 个预览环境继续保持，供管理员审核              │
│  → 目的：缓冲积压，给管理员消化时间，同时不完全停止      │
├──────────────────────────────────────────────────────────┤
│  阶段 3：完全暂停                                          │
│  待审核积压达到终极上限（默认 20，可配置）                │
│  → 停止所有新的 Claude Code 执行                         │
│  → 停止接受新需求进入执行队列                            │
│  → 系统进入暂停状态，等待管理员清空审核队列              │
│  → Review Portal 显示醒目的"系统暂停"状态横幅           │
└──────────────────────────────────────────────────────────┘
```

**设计原则**：系统允许暂停，不强求持续运转。积压是人类审核节奏的自然反馈，系统通过降速而不是强制销毁资源来响应。资源（容器、快照数据库）在管理员完成审核前始终保留，确保审核体验完整。

**阶段 3 暂停期间的需求暂存机制**

阶段 3 停止接受新需求进入执行队列，但外部输入（监控告警、Widget 反馈、GitHub Issue 等）可能在暂停期间持续产生。这些输入不能丢失：

```
阶段 3 暂停期间：
  · 外部输入仍正常接收，通过输入消毒层（Layer 1）后进入"暂存队列"
  · 暂存队列不触发 Agent 分析，仅做持久化存储
  · Review Portal 显示暂存队列的条目数，提醒管理员积压规模
  · 管理员清空审核积压、系统恢复阶段 1 后，暂存队列自动转入正常分析流程
  · 暂存队列无上限（磁盘存储），不会丢失任何输入
```

**配置项**：
```yaml
concurrency:
  max_slots: 5          # 并发上限，阶段 1 的上限
  pause_threshold: 20   # 待审核积压达到此数时完全暂停
```

---

## 8. Review Portal（管理员界面）

AutoForge 提供独立的 Web 管理后台，与目标项目解耦。

### 8.1 主要页面

| 页面 | 功能 |
|------|------|
| 需求队列 | 所有 Issue 的优先级排序列表，支持过滤/搜索 |
| 审核节点 1 | 分析报告 + 批准/拒绝 + 建议输入 |
| 审核节点 2 | 实现报告 + 双栏预览对比 + 批准/修改/拒绝 |
| Bug 审核 | 测试失败报告 + 回滚/修复决策 |
| 预览管理 | 当前所有活跃预览环境的状态和入口 |
| 执行监控 | Claude Code 实时执行日志 |
| 质量周报 | 每周巡检结果汇总 |
| 审计日志 | 全量决策和操作记录 |
| 项目设置 | `autoforge.yaml` 可视化配置 |

### 8.2 实时状态感知

Review Portal 在当前 Tauri 桌面实现中通过 Tauri event 实时感知状态变化，事件名为 `autoforge://event`。后续如需拆成远程 Web 管理后台，再把同一事件模型桥接为 WebSocket/SSE：
- Claude Code 正在修改哪个文件（实时显示）
- 预览环境是否就绪
- 测试是否通过
- 需要管理员决策时主动弹出通知

---

## 9. 规范文档体系（Normative Docs）

规范文档是 AutoForge 各环节的行为准则，**所有 Agent 都在对应规范的约束下执行工作**。规范文档从初级版本出发，随工厂运行经验逐步丰富。

### 9.1 规范文档清单

| 文档 | 作用 | 使用方 |
|------|------|--------|
| `specs/analysis-spec.md` | 需求分析规范：如何评估真实性、可行性、优先级；分析报告的结构和要素 | 需求分析 Agent |
| `specs/coding-spec.md` | 编码规范：命名、结构、安全要求；修改范围约束；初步测试要求；实现报告格式 | Claude Code Agent |
| `specs/testing-spec.md` | 测试规范：测试层级（单元/集成/端到端）；通过标准；缺陷报告格式；回归范围 | 测试 Agent |
| `specs/preview-data-spec.md` | 预览数据规范：各实体的最低数量要求；必须脱敏的字段；数据关联完整性；场景数据要求 | Preview Orchestrator |
| `specs/review-spec.md` | 审核规范：审核节点 1 和 2 的标准决策框架；管理员建议的格式和详细度期望 | 人类管理员参考 |

### 9.2 规范文档的初级版本原则

初级版本只写"最基本的约束"，避免过度规定：
- 每条规范一句话，明确"做什么"或"不做什么"
- 不规定实现细节（Agent 有判断能力）
- 遇到规范覆盖不到的情况，Agent 记录在报告中，由人类管理员决策后补充规范

### 9.3 规范文档的迭代机制

```
Agent 遇到规范未覆盖的情况
         ↓
在报告中标注"规范盲区：…"
         ↓
管理员决策后在报告中附加说明
         ↓
工厂定期将决策沉淀为规范文档更新
```

这样规范文档始终反映真实运行经验，而不是一开始就写死。

### 9.4 规范文档的规模管理（M8 后实施）

初级版本规范少、简洁，但运行半年后可能积累数百条规则。将所有规范全量注入 Agent prompt 既低效又浪费 token，"找到相关规范"本身变成一个信息检索问题。

**M8 后增加的规范管理机制：**

- **标签体系**：每条规范打标签（模块、类型、严重级别），Agent 按任务类型检索相关规范子集
- **向量检索**：规范文档嵌入向量数据库，Agent 在执行前通过语义检索拉取最相关的 N 条规范
- **规范冲突检测**：新增规范时自动检测与现有规范的语义冲突，冲突规范上报管理员裁决

初级阶段（M8 前）：规范数量有限，全量注入即可，不需要检索机制。当单个规范文档超过 50 条规则时，触发标签体系建设。

### 9.5 Vocant 项目规范文档示例（初级版本）

`specs/coding-spec.md` 初级版本要点：
- 只修改任务所需的最少文件
- 新增数据库列必须生成目标项目对应的迁移文件；若修改 AutoForge 自身数据模型，必须新增 `src-tauri/migrations/*.sql` 并由 `sqlx` 迁移执行
- 安全边界：不修改 `.env`、不修改 `alembic/versions/` 历史文件
- 所有 ID 使用 UUID 字符串，金额使用 `Numeric(15,2)`
- 实现报告必须包含：改动摘要、修改文件列表、测试情况、潜在风险

`specs/testing-spec.md` 初级版本要点：
- 必须验证核心功能路径（happy path）
- 必须验证关联模块无回归（相关接口返回 200）
- 测试失败时报告必须包含：失败位置、错误信息、复现步骤
- 外部 API（LLM、短信等）在测试中使用 mock

---

## 10. 三大核心 Agent 的定位与演化策略

AutoForge 的三个核心 Agent 是工厂质量的决定性因素。它们的最终形态不是设计出来的，而是在工厂实际运行中反复调试优化出来的。

### 10.1 需求分析 Agent

**职责**：判断"这个需求值得做吗？怎么做？"

关键能力（需调试）：
- 识别模糊、无效、重复需求的准确率
- 可行性评估与实际改动成本的一致性
- 修改方案摘要对 Claude Code 的指导价值

**初期策略**：输出结构化分析报告（固定格式），管理员在审核节点 1 看到的就是这份报告。初期报告质量低于预期时，通过补充 `analysis-spec.md` 引导 Agent 改进。

### 10.2 Claude Code Agent

**职责**：准确实现经过审核的需求，并初步验证正确性

关键能力（需调试）：
- 严格遵守编码规范和权限边界
- 修改范围最小化（不引入不必要改动）
- 初步测试能发现明显问题
- 实现报告的清晰度（管理员能快速理解做了什么）

**初期策略**：Claude Code 的能力已相对成熟，重点在于 prompt 工程——提供足够的上下文（CLAUDE.md + 规范文档 + 需求分析报告 + 管理员建议）让其做出正确判断。

### 10.3 测试 Agent

**职责**：在 Claude Code 初步测试之外，进一步评估系统整体质量

关键能力（需调试）：
- 发现 Claude Code 遗漏的问题
- 区分真实 Bug 和测试环境误报
- 主动巡检时发现隐性质量问题
- 缺陷报告的准确性（让管理员和 Claude Code 都能理解）

**初期策略**：先以"被动响应"模式运行（合并后触发），积累测试报告样本，再逐步扩展到主动巡检。

### 10.4 迭代次数软上限

每次"修改"操作的最短周期约 10-30 分钟（代码实现 + 测试 + 预览重建）。若某需求需要 10 轮迭代，管理员需要投入 2-5 小时碎片化注意力，体验会急剧恶化。

**软上限规则**：

```
迭代次数 < 3 轮：正常流程，无提示
迭代次数 = 3 轮：审核队列置顶高亮，显示迭代计数器
迭代次数 > 3 轮：系统在审核界面显示建议框：
                  "已迭代 N 轮，建议考虑：
                   · 手动介入直接修改代码
                   · 重新描述需求，提供更明确的约束
                   · 拆分为更小的子需求"
```

软上限是建议，不是强制——管理员仍可选择继续迭代。上限默认值 3 可在配置中调整。

### 10.5 调试飞轮

三个 Agent 的调试本身也是一个飞轮：
```
工厂运行产生问题（Agent 判断失误、规范不足）
         ↓
管理员决策时留下批注
         ↓
批注沉淀为规范文档或 prompt 改进
         ↓
下次运行时 Agent 表现更好
```

Agent 调试是核心工程活动，预估占总项目工作量的 50% 以上，不是"收尾工作"。里程碑规划中每个 Agent 的首次实现后都有独立的调试阶段。

---

## 11. 多项目支持与并发控制

AutoForge 可同时管理多个项目，每个项目相互隔离：

```
AutoForge Factory
  ├── Project: Vocant
  │     ├── 需求队列（独立）
  │     ├── 预览环境（独立路径）
  │     └── Claude Code 执行（独立 worktree 目录）
  │
  ├── Project: AnotherApp
  │     ├── 需求队列（独立）
  │     └── ...
  │
  └── Project: ...
```

### 11.1 并发控制机制

**核心设计：用并发槽位同时控制 Claude Code 执行和预览容器数量。**

```
全局并发槽位池（默认上限：5，管理员可修改）
  │
  ├── 槽位申请：审核节点 1 批准后，工厂为该 ChangeRequest 申请一个槽位
  │             若槽位已满，ChangeRequest 进入等待队列
  │
  ├── 槽位占用期间：
  │     · claude --bg "任务描述" 在 worktree 内并发执行
  │     · 预览容器运行，快照数据库存在
  │     · 管理员可随时查看进度
  │
  └── 槽位释放：人类在审核节点 2 批准合并 OR 拒绝丢弃后，槽位立即释放
                → 等待队列中的下一个 ChangeRequest 自动获取槽位
```

**槽位在审核期间持续占用**：Claude Code 完成后，worktree、容器、快照数据库全部保留，直到管理员完成审核决策。资源不会被超时强制销毁。

当积压过多时，系统通过背压机制（见第 7.1 节）自动降速或暂停，而不是强制清理资源。

### 11.2 并发配置

在工厂设置中可调整：

```yaml
# autoforge 全局配置
concurrency:
  max_slots: 5          # 并发上限，管理员可修改
  queue_strategy: fifo  # fifo | priority（按需求优先级）
```

预览资源消耗 = 并发数 × 单容器资源。通过控制 `max_slots` 即可控制服务器整体负载。

---

## 12. 数据模型

当前实现使用 SQLite，迁移文件位于 `src-tauri/migrations/`。所有 ID 使用 UUID 字符串；时间字段以 `datetime('now')` 生成的文本时间存储；数组/对象字段以序列化 JSON 存入 `TEXT` 字段，由 Rust model 和前端 service 层解析。

```sql
-- LLM 配置
llm_configs
  id, name, provider, model, endpoint
  api_key, ctx_window, temperature, enabled
  created_at

-- Agent 配置
agents
  id, name, name_en, role, color, initial
  llm_id                  -- FK -> llm_configs.id
  system_prompt, forge_role
  created_at

-- 项目注册
projects
  id, name, slug, description
  repo_path               -- 本地目标项目仓库路径
  branch_dev, branch_main
  config_yaml             -- autoforge.yaml 快照（TEXT）
  status                  -- active | paused | archived
  created_at, updated_at

-- 统一需求条目（当前表名为 issues）
issues
  id, project_id
  source_type             -- manual | widget | github | monitor | scan
  title, description
  category                -- Bug | Feature | Improvement | Security | Debt
  severity                -- critical | high | medium | low
  priority                -- 1-10
  status                  -- pending_analysis -> pending_review_1 -> pending_execution
                          -- -> executing -> pending_review_2 -> merged/rejected
  fingerprint             -- 去重哈希
  created_at, updated_at

-- 需求分析结果
issue_analyses
  id, issue_id
  authenticity_score      -- 外部输入使用，内部巡检默认 1.0
  feasibility_score
  priority_suggestion, category_suggestion, severity_suggestion
  duplicate_of            -- FK -> issues.id
  affected_modules        -- TEXT JSON：预估影响模块
  analysis_summary        -- ≤200 字摘要
  raw_llm_output          -- TEXT JSON：完整输出
  created_at

-- 变更请求（审核 1 通过后创建）
change_requests
  id, project_id, issue_id
  status
  admin_id, approved_at
  admin_suggestions_1     -- 审核节点 1 附加建议
  admin_suggestions_2     -- 审核节点 2 附加建议（重做时追加）
  target_branch
  created_at, updated_at

-- Claude Code 执行会话
worktree_sessions
  id, change_request_id
  worktree_path, branch_name
  status
  prompt_snapshot         -- 完整 prompt，用于审计
  iteration_count         -- 当前第几次迭代
  report_content          -- 实现报告 markdown
  started_at, completed_at

-- 预览环境
preview_environments
  id, project_id
  env_type                -- worktree | dev | main
  worktree_session_id     -- nullable（dev/main 预览时为 null）
  container_id            -- 预留：Docker/Podman 容器 ID
  preview_url             -- 访问地址
  db_snapshot_name        -- 快照数据库名（与 worktree 同生命周期）
  status                  -- pending | building | ready | failed | terminated
  created_at, ready_at, terminated_at
  -- 预留：隐私扩展字段
  data_masked_at          -- nullable，快照脱敏完成时间
  mask_policy_version     -- nullable，使用的脱敏策略版本

-- 测试执行记录
test_sessions
  id, project_id
  session_type            -- reactive | proactive
  change_request_id       -- nullable（主动巡检时为 null）
  trigger                 -- merge | scheduled | manual
  status
  summary                 -- 一句话摘要
  results_json            -- TEXT JSON：详细结果
  issues_created          -- TEXT JSON：自动创建的 issue IDs
  started_at, completed_at

-- 巡检发现
scan_findings
  id, test_session_id
  check_type
  severity, title, description
  file_path, line_number
  fingerprint
  issue_entry_id          -- 已创建 issue 时关联（FK -> issues.id）
  created_at

-- 会话与消息（管理员和 Agent 协作面板）
conversations
  id, type, name, color, initial, created_at

conversation_members
  conversation_id, agent_id

messages
  id, conversation_id, from_agent
  content_json            -- TEXT JSON：消息内容
  created_at

-- 进程内任务队列的持久化记录
job_executions
  id, idempotency_key, job_type
  payload                 -- TEXT JSON：任务参数
  status                  -- pending | waiting | running | completed | failed
  attempt, last_error
  enqueued_at, started_at, completed_at, updated_at

-- 管理员决策（完整审计链）
admin_decisions
  id, project_id, issue_id, change_request_id
  stage                   -- review_1 | review_2 | bug_review
  decision                -- approved | rejected | revision | terminated | rollback
  admin_id
  suggestions             -- 本次附加建议
  created_at
```

---

## 13. 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 桌面运行时 | Tauri 2 + Rust | 单机自托管，直接访问本地 Git 仓库、worktree、SQLite 和系统命令 |
| Review Portal | React + Tauri IPC | 前端通过 `invoke()` 调 Rust commands，通过 Tauri event 接收实时状态 |
| 工厂后端 | Rust commands + `State<AppState>` | 与 Tauri 生命周期同进程，减少本地部署复杂度 |
| 任务调度 | `tokio::mpsc` + `job_executions` | 进程内异步队列满足单桌面实例；SQLite 保留任务状态和幂等键 |
| 数据库 | SQLite + `sqlx` migrations | 本地文件数据库，WAL + foreign keys；JSON 以 `TEXT` 序列化存储 |
| 并发控制 | Rust `ConcurrencyManager` | 默认 5 个槽位、20 个审核积压阈值，可由管理员在界面调整 |
| Claude Code 集成 | 本地 Claude CLI + Git worktree | 在目标项目 worktree 中执行，AutoForge 数据库和配置不暴露给执行进程 |
| 预览环境 | 当前记录 worktree 预览 URL；后续接 Docker/Podman | 当前基础闭环先保证审核可定位到 worktree，容器化预览按 M5 增强 |
| 预览路由 | 当前本地 URL；后续 Nginx/Traefik | 单机桌面阶段无需反向代理，远程/多容器预览时再引入 |
| 通知 | Tauri notification + 后续 Webhook | 本地通知先覆盖管理员提醒，Slack/企微等外部通知后续接入 |
| 嵌入式 Widget | 纯 JS（无依赖）| 任意前端一行接入 |

---

## 14. Vocant 项目接入示例

Vocant 是 AutoForge 的第一个接入项目，也是功能验证的参照系。Vocant 的全部工厂配置集中在其仓库根目录的 `autoforge.yaml`，无需额外文档。

```yaml
# autoforge.yaml（Vocant 项目）

project:
  name: "Vocant"
  description: "AI原生业务管理工具（贸易/服务型小公司，5-20人）"
  language: python
  framework: fastapi

preview:
  build:
    command: "pip install -r requirements.txt"
    timeout: 120
  start:
    command: "uvicorn app.main:app --host 0.0.0.0 --port 8000"
    port: 8000
    health_check: "GET /health"
    ready_timeout: 30
  env:
    DATABASE_URL: "${PREVIEW_DB_URL}"
    SECRET_KEY: "${PREVIEW_SECRET_KEY}"
    LLM_API_KEY: "${LLM_API_KEY}"
  seed:
    command: "python seed_data.py"

test:
  unit:
    command: "pytest tests/unit -x --tb=short"
    timeout: 120
  integration:
    command: "pytest tests/integration -x --tb=short"
    timeout: 300

quality:
  lint: "ruff check app/"
  typing: "mypy app/ --ignore-missing-imports"

claude_context:
  key_files:
    - "CLAUDE.md"
    - "app/models.py"
    - "app/schemas.py"
  forbidden_paths:
    - ".env"
    - "alembic/versions/"     # 目标项目使用 Alembic 时，迁移文件可新增但不得删除历史
  conventions: |
    - 所有 ID 使用 UUID 字符串
    - 金额使用 Numeric(15,2)
    - 时间戳使用 now_cst()
    - 新增数据库列必须生成目标项目对应的迁移文件（此示例为 Alembic）

branches:
  dev: "dev"
  main: "main"
  worktree_prefix: "autoforge/cr-"

feedback_sources:
  - type: builtin_widget
  - type: webhook
    path: "/webhook/feedback"
```

---

## 15. 成功指标体系

AutoForge 的成功标准必须在设计阶段明确，并从 M11 开始正式采集基线数据。

| 指标类别 | 具体指标 | 目标值 | 采集方式 |
|---------|---------|--------|---------|
| 效率 | 从需求提交到 dev 合并的平均时间 | < 2 小时 | 工厂自动记录时间戳 |
| 质量 | AI 生成代码首次通过审核的比例 | > 60% | 审核节点 2 决策统计 |
| 人力节省 | 管理员每需求平均审核时间 | < 15 分钟 | Review Portal 会话时长 |
| 可靠性 | 预览系统启动成功率 | > 99% | 健康检查日志 |
| 覆盖率 | 需求无需人工重写直接可用的比例 | > 50% | 审核节点 2 的"批准"率 |
| 满意度 | 管理员对审核体验的主观评分 | > 4/5 | Review Portal 内置评分 |

这些指标同时也是调试飞轮的输入：某项指标持续低于目标值，说明对应环节的 Agent 或规范文档需要优先改进。

---

## 16. 里程碑规划

| 阶段 | 内容 | 预计工作量 |
|------|------|----------|
| M1 | Tauri 桌面骨架：项目注册 + SQLite 数据模型 + Review Portal 框架 | 5–7 天 |
| M2 | 规范文档体系初级版本 + 需求入口：限流网关 + 管理员手动提交 | 3–5 天 |
| M3 | 需求分析 Agent 首次实现 + 审核节点 1 | 5–7 天 |
| M3-T | **需求分析 Agent 调试**：真实需求样本积累 + prompt 迭代 | 5–10 天 |
| M4 | Claude Code 执行层 + **Prompt Injection 三层防护** + worktree 管理 | 7–10 天 |
| M4-T | **Claude Code Agent 调试**：端到端执行质量调优 | 7–14 天 ⚠️ |
| ✋ M4-T 中期检查点 | M4-T 启动 7 天后评估调试进展——若未达预期，调整后续里程碑时间安排再继续 | — |
| M5 | **预览系统核心增强**：容器管理 + 路径路由 + **seed 脚本数据库**（仅此路径）| 7–10 天 |
| M6 | 审核节点 2（双栏预览对比）+ 迭代软上限 + 合并流程 + 背压流控 | 5–7 天 |
| M7 | 测试 Agent 首次实现（被动响应）+ Bug 自动入队 | 3–5 天 |
| M7-T | **测试 Agent 调试**：误报率控制 + 缺陷报告质量调优 | 5–7 天 |
| M8 | 测试 Agent 主动巡检 + 定时调度 + 质量周报 | 5–7 天 |
| M9 | 多项目支持 + 资源调度 + 管理员干预能力 | 3–5 天 |
| M10 | 外部接入（Widget / GitHub Issues）+ 通知 Hub | 3–5 天 |
| M11 | Vocant 端到端验证 + 规范文档迭代 + 安全加固 + **成功指标基线采集** | 7–10 天 |
| M11+ | 生产快照路径（脱敏机制）+ Traefik 评估 + AutoForge 自身接入 | 持续 |

**总计：约 70–107 天（单人全职）**

> 里程碑分为"首次实现（Mx）"和"调试到可用（Mx-T）"两类。调试阶段估算占总工作量约 50%，不是收尾工作，是核心工程活动。M5 预览数据库初期只实现 seed 脚本路径，生产快照作为 M11+ 的增强功能。
>
> ⚠️ M4-T（7–14天）估算范围达 2 倍，反映 Claude Code Agent 调试工作量的高度不确定性——这是整个系统关键路径上风险最高的里程碑。M4-T 设有中期检查点（第 7 天），若进展不如预期及时调整，避免后续所有里程碑连锁延期。

---

## 17. 开放问题

所有关键问题已决策完毕，进入完整设计状态。

| 问题 | 决策 |
|------|------|
| AutoForge 自身的部署模式 | **自托管**（用户在自己的机器/服务器上运行） |
| 预览环境 URL 策略 | **路径路由**（`localhost:9000/vocant/cr-042`），无需泛域名证书 |
| 代码库独立性 | **独立代码仓库**，与目标项目完全解耦 |
| Claude Code 权限边界 | AutoForge 只读；目标项目 worktree 内完全权限；禁止修改 main、禁止删除 dev |
| 多轮迭代处理方式 | **软上限 3 轮**，超出后置顶高亮并显示系统建议（手动介入/重新描述），不强制终止 |
| 需求入口防护 | 限流（防恶意刷入）+ 双重审核（Agent + 人类）+ **Prompt Injection 三层防护** |
| Prompt Injection | **输入消毒层**（M4 前实现）+ **行为审计层** + **分支操作双重确认** |
| 审核积压管理 | **背压式三阶段**：正常(≤5并发) → 单线程降速(5槽位满) → 完全暂停(积压≥20)；资源始终保留，不强制销毁 |
| 预览路由实现 | 当前阶段使用本地 worktree URL；M5 接入路径路由，先用 **Nginx reload**，出现性能瓶颈时再评估 Traefik |
| M5 预览数据库 | 初期只实现 **seed 脚本路径**；生产快照（脱敏）作为 M11+ 增强功能 |
| Agent 调试工作量 | 调试阶段独立为 Mx-T 里程碑，**估算占总工作量约 50%** |
| 工厂自进化 | AutoForge 自身也是目标项目，用**固定版本的成长框架**迭代自身代码 |
| 成功指标 | 6 项可量化指标，从 M11 开始采集基线，驱动调试飞轮优先级 |
| 规范文档 | 各环节均有规范文档，从初级版本开始随运行经验迭代完善 |
| 预览数据 | 每个预览环境必须满足数据规范（由 `preview-data-spec.md` 定义） |
| 预览热更新策略 | **默认热重载**，不自动重启容器；管理员可一键手动重启 |
| 预览数据库快照 | **每次 worktree 独立快照**，随 worktree 销毁立即删除，不保留缓存 |
| Claude Code 并发策略 | `claude --bg` 并发执行，**全局槽位上限默认 5**（管理员可修改）；人类审核通过后释放槽位 |
| Widget 数据隐私 | M10 前完成：数据最小化 + 截图客户端脱敏 + 180天保留期 + IP哈希 + 删除接口；自托管部署合规由部署方负责 |
| 规范文档规模管理 | 初期全量注入；单文档超50条时触发标签体系+向量检索（M8后实施） |
| 预览资源上限 | **由并发槽位数隐式控制**：并发上限 = 最大预览容器数，无需额外资源配额机制 |
| 框架版本切换 | 完整切换协议：等待流水线清空→切换配置→验证→恢复；积压条目在新框架下重新执行 |
| Layer 1 性能 | 额外延迟1-3秒（异步处理，用户无感知）；M4实现时记录基线；成本敏感场景可换本地小模型 |
| 阶段3暂存机制 | 暂停期间外部输入进入无限容量暂存队列，系统恢复后自动转入分析流程，不丢失任何输入 |
| M4-T 中期检查点 | 调试启动第7天强制评估进展，不达预期则调整后续里程碑时间安排 |
