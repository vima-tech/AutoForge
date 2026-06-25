# AutoForge 栈品类完善实施计划（v2 · 基于真实架构梳理）

> **实施状态（已落地）**：四阶段全部实现，`cargo test --lib` 192 通过、`tsc --noEmit` 干净、`npm run build` 成功、无新增 clippy 警告。
> - 阶段 1（栈感知编码指导）：`core/stack.rs::stack_hint` 单一真源 + **双点注入**（`build_prompt` 执行阶段 + `analysis.rs::build_project_context` 分析阶段），`build_prompt` 签名未改。
> - 阶段 2（小程序检测）：`StackRole::MiniApp` + `detect_wechat_miniapp`（原生/Taro/uni-app/mpx）+ `suggest_run_config` dev_kind=miniapp + 编译闸口。
> - 阶段 3（小程序预览）：`cr_preview.rs::build_cr_miniapp`（一次性 build，不进 dev_servers/不分配端口/不探活）+ `effective_spec` miniapp kind + 前端 `Audit.tsx` 编译分支（不轮询不开浏览器）+ `services::buildCrMiniapp`。**档位 2 已实现**：`miniapp.devtools_cli_path` 设置（settings.rs get/set + Settings「小程序预览」页）+ `build_cr_miniapp` 编译成功后 `launch_devtools`（`cli open --project <产物>`，缺失/失败静默降级档位 1），结果带 `launched_devtools`。
> - 阶段 4（后端精修）：Python uv（`uv run` 前缀 + `python-uv` id）/Starlette/Sanic、Java Quarkus/Micronaut、Go `vendor/` 软链。
>
> 单测：stack.rs（taro/uniapp/native/uv/quarkus/vendor/stack_hint）+ build_prompt 栈注入 + cr_preview（miniapp effective_spec/find_miniapp_artifact）。`cargo test --lib` 200 通过。

> 目标品类：**web 前端（网站 / 后台前端）、管理后端（Java / Go / Python 等）、Tauri、微信小程序**。
> 本文档基于对 `core/stack.rs`、`commands/cr_preview.rs`、`agents/code_agent/mod.rs::build_prompt`、
> `commands/run_config.rs`、`agents/roles.rs` 的实际代码梳理，纠正了 v1 的若干错误假设。

---

## 0. 诚实的现状盘点（先承认大部分已经能跑）

把 4 个目标品类逐一对照现有代码，结论是 **3 个已基本可用，只有微信小程序是真正的全新品类**。

| 目标品类 | 现状 | 入口 | 缺口 |
|---------|------|------|------|
| **web 前端 · 网站** | ✅ 已支持 | `detect_node()` → React/Vue/Angular/Svelte/Next/Nuxt；`detect_static()` 纯静态站 | 几乎无 |
| **web 前端 · 后台前端** | ⚠️ 能检测、能跑，但 AI 不懂"后台"语义 | 同上（后台前端在技术栈上=普通前端工程） | **缺栈/领域级编码指导注入**（见 §2） |
| **管理后端 · Java** | ✅ 已支持 | `detect_java()` Maven/Gradle + Spring Boot | 框架广度（Quarkus/Micronaut，命令模板级） |
| **管理后端 · Go** | ✅ 已支持 | `detect_go()` Gin/Echo/Fiber | 框架广度（Chi/Kratos） |
| **管理后端 · Python** | ✅ 已支持 | `detect_python()` FastAPI/Django/Flask | uv 包管理；命令模板细节 |
| **管理后端 · Node** | ✅ 已支持 | `detect_node()` Express/Nest/Fastify/Koa | 几乎无 |
| **Tauri** | ✅ 完整 | `detect_tauri()` + 预览 `kind=tauri`（启动桌面程序）+ target 软链 | 无 |
| **微信小程序** | ❌ 完全不支持 | — | **检测 + 预览新 kind + worktree 编译/测试 + 编码指导**（见 §3） |

**因此本计划的两条主线是：**
- **主线 A（横切、收益最大）**：补上**栈感知的编码指导注入**——目前 `build_prompt` 只注入分析规格 + 项目 `CLAUDE.md` + `.autoforge/specs`，**没有任何"这是什么栈、该遵守什么约定"的指导**。这条缺口同时拖累后台前端、各后端框架与小程序的生成质量。
- **主线 B（纵深、唯一新品类）**：微信小程序端到端打通——检测、预览（必须新增预览 kind，无法 iframe）、worktree 内编译/测试、编码指导。

---

## 1. 关键架构事实（v1 错误已纠正）

梳理代码得到的、决定方案形态的硬约束：

1. **`core/stack.rs` 是唯一栈真源，纯 Rust 零 Tauri。**
   `detect_stacks()` 返回 `Vec<DetectedStack>`（带 `StackRole::{Frontend,Backend,Static,Desktop}`）；
   `suggest_run_config()` 合成 `RunConfigSuggestion`。其消费方已遍历：
   - `commands/run_config.rs`：预填运行配置表单（AI 再修正，人工保存）。
   - `commands/cr_preview.rs:176`：非 npm 栈的 dev 命令兜底。
   - `tasks/execution.rs:210` / `tasks/testing.rs:110`：`link_dep_caches` 软链依赖缓存进 worktree。
   - `tasks/merge.rs:366` / `execution.rs:614`：`git_add_all_args` 排除缓存软链。
   - `intake/scanner.rs:580`：`code_analyzers` 选静态分析器。

2. **运行配置是文件真源**：`.autoforge/run-config.json`（`run_config.rs:20`），经 `effective_config()` 读取。
   **不是** `dev_servers` 表——v1 计划的 `ALTER TABLE dev_servers` 是错误的，不要做。

3. **预览只有三态 `kind ∈ {web, tauri, none}`**（`cr_preview.rs::EffectiveSpec`）：
   - `web`：启动 dev server，分配端口，前端在 `http://localhost:{port}` 开浏览器/iframe（`Audit.tsx:3059`）。
   - `tauri`：直接启动桌面程序，无 iframe（`Audit.tsx:3036`）。
   - `none`：禁用预览。
   **微信小程序没有可 iframe 的本地 web server**，因此必须新增第四种 kind（见 §3.3），这是 v1 用"二维码"一笔带过、实际最难的部分。

4. **编码 Prompt 经 `agents/code_agent/mod.rs::build_prompt` 拼装**（11 参），注入顺序：
   需求工单 → 分析规格 `render_spec_brief` → codegraph 预查 → 合并需求 → 管理员建议 → 迭代提示 →
   会议室上下文 → **`.autoforge/specs`** → **项目 `CLAUDE.md`** → `autoforge.yaml` → 报告格式要求。
   **没有任何按栈/框架注入的指导段**——这是主线 A 的落点。

5. **`agents/roles.rs` 是会话/流水线角色注册表**（meeting、analysis 等 persona），
   **不是编码栈指导的入口**。v1 提议在此加 `wechat_taro_prompt` 是放错层——会被
   `build_prompt` 完全绕过。正确机制是 §2 的 `code_agent_skills` / `.autoforge/specs`。

6. **`framework` / `language` 字段的真实消费方**：`commands/deploy.rs:293/298`（部署目标推断）+ 分析提示。
   对 **Node 系栈（含所有前端框架、Taro/uni-app 小程序），dev/build/test 命令来自 `package.json` scripts 而非框架名**——
   所以"给 `frontend_framework()` 多加 Remix/Astro/Qwik 名字"几乎不改变行为，是低价值填充，本计划不做。
   框架识别只在**命令模板确实因框架而异**处才有价值（后端 Java/Go/Python）。

7. **已有可复用机制：`code_agent_skills` 表（迁移 0067）支持 `project_id` 作用域**（NULL=全局）。
   `code_agent/mod.rs::load_code_agent_skills` 已把它注入 worktree（claude 写 `.claude/skills`、
   codex/opencode 折叠进 prompt）。**这是栈/领域编码指导的现成载体**，主线 A 直接复用，不另造轮子。

---

## 2. 主线 A：栈感知的编码指导注入（横切，优先做）

### 2.1 问题

`build_prompt` 给编码 Agent 的上下文里，**没有"你正在改一个什么样的工程、该守什么约定"**。
后果：后台前端不知道该用项目既有的 antd/Pinia 模式；FastAPI 项目可能写成同步阻塞；
小程序项目可能误用 `document`/`window`。靠每个项目手写 `CLAUDE.md` 能缓解，但**新接入项目零配置时质量塌方**。

### 2.2 方案：两层，优先复用既有机制

**第一层（必做、零迁移）：在 `build_prompt` 注入一段「栈画像 + 默认约定」。**
- 调 `core::stack::detect_stacks(repo_path)`，把检测到的栈摘要（id/role/language/framework）写进 prompt，
  紧跟在"项目规范"段之前，标题如 `## 技术栈画像（自动检测）`。
- 为每个 `StackRole`/语言挂一段**精炼的默认约定**（纯字符串常量，放 `core/stack.rs` 或新建
  `agents/code_agent/stack_hints.rs`，保持纯 Rust）。例如：
  - 后台前端（Frontend + 检测到 antd/element/arco）：列表页/表单页/权限/请求层分离的默认骨架约定。
  - FastAPI：async/await、依赖注入、pydantic 模型。
  - Django：app 结构、ORM 迁移、`manage.py` 约定。
  - 小程序（见 §3.4）。
- **关键克制**：这段是"默认猜测"，明确声明"项目 `CLAUDE.md`/`.autoforge/specs` 若有冲突以其为准"，
  避免与项目自有约定打架。篇幅每栈 ≤ 15 行，防 prompt 膨胀。

**第二层（可选、按需）：项目级深度约定走既有 `code_agent_skills` / `.autoforge/specs`。**
- 不新增表、不动 roles.rs。需要更细的领域约定时，用 Settings「编码技能」CRUD 建项目作用域 skill，
  或在仓库写 `.autoforge/specs/*.md` —— 两者 `build_prompt` 已经会注入。
- 可提供**一键生成**：检测到栈后，让 analysis/doc_writer 角色生成一份建议的 `.autoforge/specs/stack_conventions.md` 草稿，人工审后入库。

### 2.3 注入点：必须**双点**（分析 + 执行），不止执行

再评估时核对代码发现：栈画像不能只注入执行阶段，**分析阶段更要紧**。
- **分析阶段** `agents/analysis.rs::build_project_context`（:475）：注入 README/CLAUDE.md/`.autoforge/specs`/
  目录树/git log，但**没有蒸馏过的栈画像**（只丢原始 config + 目录树让 LLM 自己猜）。
  分析产出 `affected_files`/`suspected_locations` 的**文件路径**，编码 Agent 直接信任——
  若分析没认出 Taro 工程，会给出错误的文件约定（如建议改 `.vue` 而非 `pages/x/index.tsx`）。
- **执行阶段** `agents/code_agent/mod.rs::build_prompt`（:365）：同样缺画像。

两处都注入同一段「技术栈画像 + 默认约定」（同一 `stack_hints` 来源），保证分析与执行对栈的认知一致。

### 2.4 改动点

| 文件 | 改动 | 备注 |
|------|------|------|
| `agents/code_agent/stack_hints.rs`（新） | 按 role/language/framework 返回默认约定字符串 | 纯 Rust、零 Tauri |
| `agents/analysis.rs::build_project_context` | 注入「技术栈画像」段（async 函数，可直接调 `detect_stacks`） | 分析阶段注入点 |
| `agents/code_agent/mod.rs::build_prompt` | 同段注入 | 执行阶段；11 参签名多处调用，**只在函数体内自取，不改签名** |
| `core/stack.rs` | 暴露 `stack_summary_line()` / `stack_hint(stacks)` 供两处复用 | 单一真源，避免分析/执行漂移 |
| 单元测试 | 画像段含栈摘要 + 冲突让位声明 | 复用现有 `build_prompt_emits_report_marker` 模式 |

> **铁律**：`build_prompt` 已有 11 个参数且被 execution/orchestration/merge 多处调用，
> 新逻辑一律在函数体内自取（`detect_stacks(repo_path)`），**不要再加参数**。

---

## 3. 主线 B：微信小程序端到端

### 3.1 检测（`core/stack.rs` 新增 `detect_wechat_miniapp`）

小程序工程有两类形态，检测需都覆盖：

| 形态 | 判定标志 | 栈 id | dev/build 来源 |
|------|---------|-------|---------------|
| 原生小程序 | `project.config.json` + `app.json` + 无构建框架 | `wechat-native` | 无 npm scripts，命令=开发者工具编译 |
| Taro | `package.json` deps 含 `@tarojs/taro` / `@tarojs/cli` | `wechat-taro` | `package.json` scripts（`build:weapp`/`dev:weapp`） |
| uni-app | deps 含 `@dcloudio/uni-app` 或有 `manifest.json`+`pages.json` | `wechat-uniapp` | scripts（`dev:mp-weixin`/`build:mp-weixin`） |
| mpx | deps 含 `@mpxjs/core` | `wechat-mpx` | scripts |

- **新增 `StackRole::MiniApp`**（不要复用 Frontend——预览/命令/产物语义都不同）。
- Taro/uni-app/mpx 都是 Node 工程：包管理器、`node_modules` 软链、scripts 解析**全部复用 `detect_node` 的现成逻辑**，
  只是 role 改 MiniApp、命令脚本名优先匹配小程序专用脚本（`build:weapp`/`dev:mp-weixin` 等）。
- 原生小程序无 scripts：命令字段留空，dev 走"开发者工具编译"（见 §3.3）。
- **检测优先级**：`detect_wechat_miniapp` 要排在 `detect_node` 之前（否则 Taro 会被当普通前端），
  在 `detect_stacks()` 里与 Tauri 同级先于 `detect_node` 短路。

### 3.2 worktree 内的编译/测试（execution + testing）

- 依赖软链：Taro/uni-app 的 `node_modules` 已被 `dep_cache_dirs`（继承 `detect_node`）覆盖，零额外工作。
- 合并前测试门：`test_unit` 取 scripts 的 `test`（如 vitest/jest），`build_command` 取 `build:weapp` 等——
  **编译通过即作为最低质量闸口**（小程序无浏览器 e2e）。
- 安全/分析器：复用 `npm_audit` + `eslint`。

### 3.3 预览：一次性 build 动作（不是第四种"server"，这点 v2 仍低估）

微信小程序**没有可 iframe 的 localhost web server**。再评估读 `start_cr_preview`（cr_preview.rs:380）后
发现一个更深的问题：**现有预览整套机制是围绕"持久 dev server"建的**——
spawn 长驻进程塞进 `state.dev_servers` map、分配端口注 `PORT`、前端轮询 `url_reachable`
判 `starting→running`、`preview_environments` 标 `building`。
而小程序编译是**一次性命令**（`build:weapp` 跑完即退出）。若把它塞进 `start_cr_preview` 的持久路径，
进程一退出，前端探活立刻判为"崩溃/未就绪"——语义完全错位。

**正确建模：小程序预览 = 一次性 build 动作 + 日志流**，生命周期上更接近 `tasks/testing.rs` / `execution.rs`
的"跑命令到结束、抠退出码、流式日志"，而非 dev_server 的"长驻 + 探活"。因此**不要复用
start_cr_preview 的持久 handle 路径**，应另开一条命令（如 `build_cr_miniapp`），或在 cr_preview 内
明确分叉一条"run-to-completion，不 insert dev_servers map、不分配端口、不探活"的分支。

```
preview kind = "miniapp"   // 第四态，但语义是"可编译产物"而非"可访问 URL"
状态机：idle → building → built(产物路径) / failed(退出码 + 日志)
```

前端 `Audit.tsx::renderCrLaunch` 加 `kind === 'miniapp'` 分支，**不轮询 reachability、无"打开浏览器"**：

- **档位 1（默认、零外部依赖）· 编译产物**
  - 后端一次性跑 `build:weapp`（或原生编译），流式回传日志（复用 `PreviewLog`/`LiveLogModal`），
    退出码 0 → 回传产物目录（`dist/weapp` / `dist/build/mp-weixin`）。
  - 前端展示：编译状态(building/built/failed) + 产物路径 + "用微信开发者工具打开此目录"指引 + 实时日志。
  - 不自动登录/扫码——确定性优先，绝不引入会卡住的人工交互。
- **档位 2（可选、需本机装开发者工具）· CLI 拉起**
  - Settings 配了开发者工具 CLI 路径则编译后 `cli open --project <产物目录>`（或 `preview` 出二维码图落临时文件回传）。
  - CLI 缺失/失败 → 自动降级档位 1，不报硬错。

**改动点（预览）**：
| 文件 | 改动 |
|------|------|
| `commands/cr_preview.rs` | 新增 `build_cr_miniapp`（或 miniapp 分叉）：**run-to-completion**，不 insert `dev_servers`、不分配端口、不探活；流式日志 + 退出码 + 产物路径 |
| `commands/cr_preview.rs::EffectiveSpec` / `get_cr_preview` | `kind` 增加 `"miniapp"`，状态语义 building/built/failed（非 reachability） |
| `src/services/index.ts` | 封装 `build_cr_miniapp`（IPC 单一入口铁律） |
| `src/pages/Audit.tsx::renderCrLaunch` | `kind === 'miniapp'` 分支：编译按钮 + 产物路径 + 日志，**不轮询 reachability、无浏览器** |
| `src-tauri/capabilities/main.json` | 声明新命令权限 |
| Settings（可选档位 2） | 微信开发者工具 CLI 路径，存 `app_settings`（`read_setting/write_setting`，**非 dev_servers 表**） |

### 3.4 小程序编码指导（接主线 A 第一层）

在 `stack_hints.rs` 为 `wechat-*` 栈挂默认约定：
```
# 微信小程序（Taro/原生）
- 禁止浏览器 API：不得用 document / window / fetch / localStorage；
  改用 Taro.* / wx.*（导航 Taro.navigateTo、存储 Taro.setStorage、请求 Taro.request）。
- 页面结构：原生=page/{name}.{js,wxml,wxss,json}；Taro=pages/{name}/index.tsx + index.config.ts。
- 样式：rpx 单位、scss module；组件走小程序自定义组件或 Taro 组件。
- 网络：统一经 service 层封装 Taro.request，自动注入登录 token。
- 改动后必须 `build:weapp`（或对应脚本）编译通过。
```
更细的项目约定仍走 §2 第二层（`.autoforge/specs` / 项目作用域 skill）。

---

## 4. 后端/前端的小幅精修（命令模板级，低优先）

只做"命令模板确实因框架而异"的部分，**不做纯框架改名的填充**：

| 栈 | 精修 | 价值 |
|----|------|------|
| Python | 检测 `uv.lock` → 命令前缀 `uv run`（替代 `poetry run`）；补 Starlette/Sanic 的 dev 命令 | 实际改变命令，有价值 |
| Java | 检测 Quarkus（pom/gradle 含 `quarkus`）→ `quarkus:dev`/`gradlew quarkusDev`；Micronaut → `gradlew run` | dev 命令因框架而异，有价值 |
| Go | 检测 `vendor/` 存在 → 加入 `dep_cache_dirs` 软链（加速 worktree 编译） | 实际加速，有价值 |
| 后台前端 | 走主线 A 的 `stack_hints`，但**只陈述可验证事实**（"项目依赖含 antd"），**不臆断"这是后台"、不强加管理台骨架** | 见下方诚实性说明 |

> **"网站 vs 后台前端"无法靠依赖可靠区分**（营销站也用 antd/element）。再评估时纠正：hint 只说
> "检测到 UI 库 X"这类可验证信息，是否后台、用什么骨架，由项目自己在 `.autoforge/specs` 声明，
> 不在检测层做不可靠的领域分类。这是相对前几版"检测 antd→判定后台"的纠偏。

> 明确**不做**：给 `frontend_framework()` 增加 Remix/Astro/Qwik/Solid——对 Node 栈命令来自 scripts，
> 加名字不改行为，属低价值。若将来 deploy.rs 需要按这些框架做差异化部署，再按需补。

---

## 5. 分阶段实施（按收益/依赖排序）

### 阶段 1 · 栈感知编码指导（主线 A，1 周）
- [ ] `agents/code_agent/stack_hints.rs`：role/language/framework → 默认约定字符串（含小程序）
- [ ] **双点注入**：`build_prompt`（执行）+ `analysis.rs::build_project_context`（分析）注同一段（函数体内自取 `detect_stacks`，**不改 build_prompt 签名**）
- [ ] 冲突让位声明 + 篇幅控制单测
- **交付**：所有品类（含现有后端/后台前端）零配置时即获得默认约定；分析与执行对栈认知一致，分析产出的文件路径不再错。**无迁移、无前端改动**，风险最低，先落地验证。

### 阶段 2 · 微信小程序检测 + 编译闸口（主线 B 上半，1.5 周）
- [ ] `StackRole::MiniApp` + `detect_wechat_miniapp`（原生/Taro/uni-app/mpx）
- [ ] `detect_stacks` 优先级（MiniApp 先于 node）+ 单测 4 条
- [ ] `suggest_run_config` 对 MiniApp 产出 build/test（编译即闸口）
- [ ] 小程序 hint 接入阶段 1 的 `stack_hints`
- **交付**：能识别小程序工程、AI 按小程序约定生成代码、合并前以"编译通过"为闸口。**无前端预览改动**（预览在阶段 3）。

### 阶段 3 · 微信小程序预览：一次性 build 动作（主线 B 下半，1.5 周）
- [ ] `cr_preview.rs` 新增 `build_cr_miniapp`（或分叉）：**run-to-completion**，不 insert `dev_servers`、不分配端口/不探活；流式日志 + 退出码 + 产物路径（档位 1）
- [ ] `get_cr_preview` `kind="miniapp"` 状态语义 building/built/failed（**非 reachability**）
- [ ] `services/index.ts` 封装 + `capabilities/main.json` 权限声明
- [ ] `Audit.tsx::renderCrLaunch` miniapp 分支（编译/产物/日志，**不轮询 reachability、无浏览器**）
- [ ] （可选）Settings 微信开发者工具 CLI 路径（存 `app_settings`）+ 档位 2 拉起，缺失自动降级
- **交付**：审核页可一键编译小程序、查看产物与日志、（可选）拉起开发者工具。**web/tauri 预览零回归。**

### 阶段 4 · 后端/Go 命令模板精修（低优先，1 周）
- [ ] Python uv 支持；Java Quarkus/Micronaut；Go vendor 软链
- [ ] 各自单测
- **交付**：后端框架广度补齐到命令层。

> 阶段 1 完全独立、零迁移、零前端，**建议立即开工**作为最小验证。阶段 2→3 串行（预览依赖检测）。阶段 4 可随时并行插入。

---

## 6. 迁移与数据模型（纠正 v1）

- **小程序检测本身零迁移**：`detect_stacks` 返回值结构不变，新增 `StackRole::MiniApp` 不碰 DB。
  再评估已核实 **`StackRole` 仅在 `core/stack.rs` 内部使用**（全仓外部零引用），新增变体只需补
  `as_str()` / `rank()` / `suggest_run_config` 内几处 match，改动面小、无扩散。
- **预览状态零迁移**：预览按需启动，状态不落表；档位 2 的 CLI 路径走 `app_settings`（`read_setting/write_setting`），**不 ALTER dev_servers**（该表的角色与此无关）。
- **编码指导零迁移**：默认约定是 Rust 常量；项目级深度约定复用既有 `code_agent_skills`（0067）/ `.autoforge/specs`，无新表。
- 仅当将来要**持久化"项目选定的小程序平台/框架"**时才考虑加列——当前由 `detect_stacks` 每次嗅探即可，无需落库。

> 即：本计划**主体无需任何新迁移**。这是相对 v1（凭空加 3 个迁移）的重要纠正。

---

## 7. 风险与缓解（基于真实约束）

| 风险 | 真实根因 | 缓解 |
|------|---------|------|
| 小程序无法浏览器预览 | 预览模型只有 web(iframe)/tauri(app) | 新增 `kind=miniapp`，档位 1 只做"编译+产物+日志"，确定性优先；不强求扫码 |
| 开发者工具需登录/扫码会卡流水线 | 微信生态强制人工登录 | 预览只在**审核页人工触发**，不进无人值守流水线闸口；闸口用"编译通过" |
| `stack_hints` 与项目自有约定打架 | 默认猜测可能过时/不符 | hint 段显式声明"项目 CLAUDE.md/.autoforge/specs 优先"；篇幅 ≤15 行/栈 |
| `build_prompt` 多调用点 | 已 11 参，改签名会扩散 | 新逻辑在函数体内自取 `detect_stacks`，**绝不加参数** |
| 框架填充虚假繁荣 | Node 栈命令来自 scripts | 明确不加无行为差异的框架名，避免维护负担 |
| 跨平台软链 | symlink 仅 unix | 沿用 `link_dep_caches` 既有 `#[cfg(unix)]` 策略，CI 在 Linux 验证 |

---

## 8. 验收标准

**单元测试（`core/stack.rs`）**
```rust
detect_wechat_miniapp: Taro(@tarojs/taro)→wechat-taro/MiniApp；
                       uni-app(@dcloudio/uni-app)→wechat-uniapp；
                       原生(project.config.json+app.json)→wechat-native；
                       优先级：Taro 不被 detect_node 抢成 frontend。
suggest_run_config: 小程序 build_command 取 build:weapp/build:mp-weixin。
python uv: uv.lock 存在→命令前缀 uv run。
java quarkus: pom 含 quarkus→dev_command 含 quarkus:dev。
```
**单元测试（`build_prompt`）**：含「技术栈画像」段 + 冲突让位声明。

**集成验证**
```
后台前端：建 React+antd 项目→检测→提交列表 CRUD 需求→生成代码含 service/权限分层。
小程序：建 Taro 项目→检测 wechat-taro→提交需求→生成代码无 document/window→build:weapp 编译通过。
预览：审核页对小程序 CR 一键编译→见产物路径+实时日志；web/tauri 预览不回归。
```
**代码审核铁律**
```
✅ stack.rs / stack_hints.rs 纯 Rust，无 tauri::*
✅ build_prompt 未加参数（函数体内自取栈）
✅ 无新迁移（或新迁移可回滚、checksum 正确）
✅ 预览 web/tauri 两态行为零回归
```

---

## 9. 与 v1 计划的差异说明（避免回退）

| v1 的做法 | 为何错 | v2 纠正 |
|----------|--------|---------|
| 在 `roles.rs` 加 `wechat_taro_prompt` | roles 是会话角色，被 build_prompt 绕过 | 走 `stack_hints` + `code_agent_skills`/`.autoforge/specs` |
| `ALTER TABLE dev_servers` 存预览模式 | run_config 是**文件**，预览状态不落表 | 零迁移；CLI 路径走 `app_settings` |
| 加 Remix/Astro/Qwik 等框架名 | Node 栈命令来自 scripts，加名不改行为 | 不做；只精修命令因框架而异的后端 |
| 3 个新迁移 | 均无结构必要 | 主体零迁移 |
| 小程序预览"二维码"一笔带过 | 没识别预览模型只有 web/tauri | 新增第四 kind + 双档位 + 自动降级 |
| 5 Sprint 框架大铺开 | 大量低价值填充 | 4 阶段，主线 A（横切）先行，聚焦真实缺口 |
