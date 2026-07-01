# AutoForge 双头架构实施文档（Tauri + Web 共用 core）

> 目标：把后端拆成 **core（纯 Rust）+ Tauri 对接层 + Web 对接层**，两个对接层尽量轻薄，
> 只做「认身份 · 转请求 · 桥事件 · 供宿主能力」四件事。**新增/修改业务只改 core 一处，
> 两个对接层一行不改**。同时为多用户/权限预留一致的注入点，使桌面单用户 owner 与团队内网多用户协作同构——目标是**团队协作**（通常内网/局域网访问），不是面向不特定客户的多租户 SaaS。
>
> 本文档基于对当前 `dev`（0080 迁移基线）的实测核查撰写，关键结论均附 `文件:行` 证据。
> 状态：**设计 + 实施计划，尚未动工**。落地顺序见 §8；分轨推进与拆分建议见 §12；代码组织拆分见 §11。

---

## 0. 文档性质

- 这是一份 **实施蓝图**，不是最终代码。每个改造项标注了涉及文件、机制、工作量档位（S/M/L）与验收点。
- 记法：`S`≈半天内、`M`≈1–3 天、`L`≈1 周量级（按单人估）。
- 「两形态」贯穿全文，**目标场景是团队内网协作，不是公网 SaaS**：
  - **形态 A｜同进程双头**：Tauri 进程内顺手起一个 axum，浏览器访问 localhost/局域网。单进程、本地能力全保留，适合"先跑通、验证需求"。
  - **形态 B｜团队内网 headless 部署**：后端脱离 Tauri、单独部署到团队内网一台常驻机器（办公室闲置主机/NAS/小型 Linux box），团队成员通过内网浏览器访问同一个实例——本质上和形态 A 是同一套环境（一套代码仓库、一份 `claude` CLI 授权、一条构建链），只是从"一个 GUI 窗口"变成"多个浏览器连同一个后端"，**不涉及多租户隔离、计费或公网访问**。
  - §1–§5 的改造 **A/B 共用**；B 专属的只是「团队登录与角色」（§5），用于回答"团队内谁能做什么"，不是"如何隔离互不信任的客户"。

---

## 1. 现状评估（经代码核查）

### 1.1 有利条件——地基已就位

| 结论 | 证据 | 意义 |
|------|------|------|
| **前端 IPC 单一入口** | `src/services/index.ts` 是唯一 `invoke` 出口；页面/组件里 **0 处**直接 `invoke` | 请求收敛天然完成，改 `ipc()` 一处即切换传输 |
| **前端 0 处使用 Tauri 插件 API** | 全仓 `grep` 无 `@tauri-apps/plugin-*`、无 `notification/dialog/fs/path/shell` 前端调用 | Web 头前端无需替换插件调用，只剩窗口 chrome 与事件订阅两处 |
| **命令提取器只有两种** | `State<'_, AppState>` 295 处、`app: AppHandle` 36 处；**无 `Window`/`Webview`/`Request` 提取器** | 统一 `Ctx` 契约可 **零例外**覆盖全部命令 |
| **AppState 纯净** | `src-tauri/src/state.rs:17` `AppState` 仅含 `db/job_tx/concurrency/dev_servers/webhook_handle/asr_sessions/autosupply_running`，无 Tauri 类型 | 可直接 `Arc<AppState>` 供两层复用 |
| **事件单一出口** | `src-tauri/src/core/event.rs:166` `emit(app,&AppEvent)`；`AppEvent` 全 `Serialize` | 抽 `trait EventSink` 只改一处出口，SSE 序列化零成本 |
| **路径走 OnceLock** | `state.rs` `init_*_base` / `worktrees_base()` 等，非 Tauri 入口也能初始化 | headless 启动可自行喂路径 |
| **密钥已支持无钥匙环回退** | `src-tauri/src/core/secrets.rs:113` `load_or_create_master_key`：`keyring` 失败→`0600` 文件（`secrets.rs:168`）；入口 `init_secrets(master_key_file)` (`secrets.rs:44`) | headless 无钥匙环自动降级，**无需改 secrets.rs**，对接层喂路径即可 |
| **审批已留操作者字段** | `change_requests.rs` `record_admin_decision*(... admin_id ...)`（如 `:1100`） | 多用户接入点已存在，Principal 接上即有审计 |

### 1.2 三个结合点（量化）

| 结合点 | 规模 | 改造性质 |
|--------|------|----------|
| 命令注册 | **312** 个 `#[tauri::command]`，全部薄包装 | 收敛为「注册表 + 单 dispatch」 |
| 事件透传 | `event::emit` 调用 **78** 处；`AppHandle` 在函数签名透传 **49** 处；命令签名带 `app: AppHandle` **36** 处 | 机械替换为 `Arc<dyn EventSink>` |
| 前端传输 | `invoke` 仅在 services；`listen('autoforge://event')` 散在 **6** 个页面（App/Dashboard/Projects/Settings/Conversations/Audit） | `ipc` 已收敛；`listen` 需收口为 `subscribe()` |

### 1.3 评估发现的现实约束（原始设计漏点，必须在实施中正面处理）

1. **`app.path()` 有 3 类用途**——`app_data_dir`（`lib.rs:36` 启动定 data 目录）、`download_dir`+`temp_dir`（`issues.rs:452`、`intake.rs:313` 导出落盘）。headless 无 `PathResolver`，**这些路径必须由对接层在启动时注入**（并入 §5.6 启动配置）。
2. **导出是「后端写文件到 `download_dir`」模式**，不是把字节返回前端（`issues_export_xlsx` 返回 `Vec<u8>` 后由命令落盘）。Web 头下「服务器磁盘的下载目录」对浏览器用户无意义 → **这类命令是真正需要两头行为不同的例外**（Tauri=落盘+可 reveal；Web=HTTP 附件下载）。登记于 §7。
3. **`notification`/`shell` 插件是死注册**——`lib.rs:31-32` 注册了，但前端 0 调用、后端 `Command::new` 全走 `std/tokio::process`（非 `ShellExt`）。→ 宿主能力抽象**只需覆盖 opener**，无需 OS 通知/ shell 抽象；桌面通知走已有 DB 通知收件箱即可。
4. **`reveal_item_in_dir` / `open_path` 在 Web 头无对应语义**（`artifacts.rs:253/258`、`backup.rs:411/416`）——服务器文件系统 ≠ 用户桌面。Web 实现只能**降级为下载或直接禁用**，须在能力 trait 里显式标注。
5. **多用户完全从零**——无 `users`/`auth` 表或模型（仅有的 `password` 是 `git_password`/backup/clawbot，与账户无关）。好处是无历史包袱；代价是**现有查询全无 user 过滤**。目标场景是团队共享一个部署（非多租户隔离），但即便同一个团队也可能需要「谁能碰哪个项目」的轻量 ACL（如外部顾问只接触自己负责的项目），建议在 core 预留 `scope(ctx)` 过滤层（§5.4）、默认恒等，需要时再收窄，避免未来要扫全部 sqlx。

### 1.4 商业与组织现实约束（本轮补充评估）

基于 `git shortlog`/`git log` 核实的项目现状，直接影响 Track 2 的推进方式（详见 §12）：

- **单人人类维护**：提交历史里仅 `Renmengkai`（150 次，`renmengkai@gmail.com`）是人类作者；另一账号 `AutoForge`（39 次，`autoforge@local`，提交信息带 `[autoforge #<hash>]` 标记）核实为项目自身编码 Agent 经 CR 流水线的自动化提交（squash merge 产物）——是很好的自举（dogfooding）证明，但**不是第二位人类维护者**。当前组织能力 = 1 人。
- **项目极年轻、极高速**：186 个提交集中在 2026-05~06 两个月内，功能面已横跨十余个子系统（自动供料、多源搜索、编码 Agent 进群聊、孵化台……），近乎日更节奏。
- **直接结论**：Track 2 的目标是**团队内网协作**（非公网 SaaS），所需组织能力远低于最初评估——不涉及合规、计费、多租户安全隔离、7×24 公网值守；主要是"有没有一台常驻在线的内网机器、谁负责它"这类基础设施问题，1 人也能承担。这条现实约束修正了 §12 的判断：Track 2 的门槛比表面看起来低得多。

---

## 2. 目标架构

```
┌───────────────── autoforge-core（纯 Rust crate：零 tauri / 零 axum）──────────────────┐
│ agents/ core/ tasks/ models/ db/ state/                                                │
│ 业务命令：#[command(perm="…")] async fn foo(ctx:&Ctx, a:X, b:Y) -> Result<Z, AppError> │
│   └─ 宏在编译期把每条命令登记进 CommandRegistry（唯一真源）                              │
│ 抽象出口：trait EventSink · trait HostCapabilities · Principal · fn authorize()         │
│ 二进制出口：#[blob_command] fn(..) -> Result<Blob>（Blob{bytes,mime,filename}）          │
└─────────────┬───────────────────────────────────────────────────────┬─────────────────┘
      Tauri 对接层（薄·不枚举命令）                              Web 对接层（薄·不枚举命令）
   · 1 个 #[tauri::command] dispatch → registry            · POST /rpc/:cmd → registry
   · TauriSink(AppHandle) 桥 app.emit                      · GET  /events (SSE) ← broadcast
   · TauriHost(opener 插件 / path resolver)                · GET  /blob/:cmd 流式下载
   · Principal::local_owner()（或本地 PIN）                 · WebHost（open_external→前端 window.open）
                                                           · 登录端点 + Principal 提取器（session/JWT）
             │                                                            │
       桌面 WebView ◄── services 层按 __TAURI_INTERNALS__ 运行时切传输 ──► 浏览器
```

### 最高约束（判定「对接层够不够薄」的红线）

> **对接层不得枚举具体命令名。** 一旦对接层里出现 `match cmd { "list_projects" => … }` 或 312 条命令清单，即不合格。对接层只允许做四件事：**认身份 · 转请求 · 桥事件 · 供宿主能力**。加/删业务命令时，两个对接层的 diff 必须为 0。

---

## 3. 核心机制：命令注册表 + 统一 Ctx 契约

这是「改一处」得以成立的发动机。业务代码继续写**强类型舒适签名**，宏负责生成「类型擦除」的统一 handler 并登记到全局注册表。

### 3.1 统一契约

```rust
// core::rpc
pub struct Ctx {
    pub state: Arc<AppState>,             // 现有纯字段，零改
    pub sink:  Arc<dyn EventSink>,        // 取代到处透传的 AppHandle
    pub host:  Arc<dyn HostCapabilities>, // opener 等宿主能力（§6）
    pub principal: Principal,             // 调用者身份（§5）
}

pub type Handler =
    fn(&Ctx, serde_json::Value) -> futures::future::BoxFuture<'_, Result<serde_json::Value, AppError>>;

pub struct CommandDef {
    pub name: &'static str,
    pub perm: Option<&'static str>,       // 声明式授权点（§5.3）
    pub handler: Handler,
    pub blob: bool,                        // 是否二进制通道
}
```

### 3.2 `#[command]` 宏（业务侧唯一要写的形状）

```rust
// 业务代码——core 里唯一需要新增/修改的地方
#[command(perm = "project.delete")]
async fn delete_project(ctx: &Ctx, id: String) -> Result<(), AppError> {
    // …原逻辑；用 ctx.state / ctx.sink / ctx.principal…
}
```

宏在编译期展开出两样东西：
1. **擦除 handler**：内部 `serde_json::from_value` 拆 args、调强类型 fn、`to_value` 回包，`AppError` 统一映射。
2. **注册项**：`inventory::submit!{ CommandDef { name:"delete_project", perm:Some("project.delete"), handler, blob:false } }`（用 `inventory` 或 `linkme` 做编译期收集；运行时 `CommandRegistry::global()` 一次性建 `HashMap<&str, &'static CommandDef>`）。

> 参数约定：命令入参统一为一个可 `Deserialize` 的结构或裸参数（宏支持多参→内部合成 `struct`）。前端传参从当前 Tauri 的 camelCase 习惯，统一收敛为 **snake_case JSON body**（Tauri dispatch 侧做一次兼容映射，见 §4）。

### 3.3 渐进迁移策略（关键——不要求一次改完 312 个）

- 新增 `#[command]` 宏与旧 `#[tauri::command]` **并存**。已迁移命令进注册表、走 dispatch；未迁移命令仍走旧 `generate_handler!`。
- Tauri dispatch 命令对「注册表未命中」的 cmd 回落到旧路径，实现**逐个命令灰度迁移、全程可编译可运行**。
- 迁移完成后再删除 `generate_handler!` 长清单。

---

## 4. 改造项清单

| ID | 改造 | 涉及文件 | 机制 | 档位 |
|----|------|----------|------|------|
| **R1** | 事件 sink 抽象 | `core/event.rs` + 49 处 `AppHandle` 透传点 | 定义 `trait EventSink`；`emit(&dyn EventSink, ev)`；`TauriSink(AppHandle)` 保留 `app.emit` 与通知收件箱副作用（`event.rs:172`）；机械替换透传签名 | **M** |
| **R2** | Ctx + `#[command]` 宏 + 注册表 | 新增 `core/rpc/`（宏可放独立 proc-macro crate） | §3；`inventory` 收集 | **M** |
| **R3** | 业务命令迁移到统一契约 | `commands/*.rs` 逐个 | 强类型 fn 收 `&Ctx`；渐进灰度（§3.3） | **L**（可分批） |
| **R4** | Tauri 对接层瘦身 | `lib.rs` | 保留 1 个 `dispatch` 命令 + blob 命令；`generate_handler!` 逐步清空 | **S** |
| **R5** | Web 对接层（新 bin/模块） | 新增 `src-tauri/src/bin/web.rs` 或独立 crate | axum：`POST /rpc/:cmd`、`GET /events`(SSE)、`GET /blob/:cmd`、登录端点 | **M** |
| **R6** | 宿主能力 & 例外通道 | `core/host.rs`（新）+ `demo.rs`/`artifacts.rs`/`backup.rs`/`issues.rs`/`intake.rs` | `trait HostCapabilities`（§6）；导出改 blob 通道（§7） | **M** |
| **R7** | 前端传输自适应 | `src/services/index.ts` + 新增 `subscribe()` + `src/lib/window.ts` + 6 个 `listen` 页面 | 见 §4.3 | **M** |

### 4.1 Tauri 对接层最终形态（几十行，永不随业务增长）

```rust
#[tauri::command]
async fn dispatch(cmd: String, args: serde_json::Value,
                  state: State<'_, Arc<AppState>>, app: AppHandle)
    -> Result<serde_json::Value, String> {
    let ctx = Ctx {
        state: state.inner().clone(),
        sink:  Arc::new(TauriSink(app.clone())),
        host:  Arc::new(TauriHost::new(app.clone())),
        principal: Principal::local_owner(),        // 桌面默认全权（可换本地 PIN，§5.1）
    };
    CommandRegistry::global().call(&cmd, &ctx, args).await.map_err(|e| e.to_string())
}
// invoke_handler 里最终只注册 dispatch（+ blob dispatch）
```

### 4.2 Web 对接层（axum，路由不枚举命令）

```rust
async fn rpc(Path(cmd): Path<String>,
             principal: Principal,                 // extractor：解析 session/JWT（§5.1）
             State(app): State<Arc<AppState>>,
             Json(args): Json<serde_json::Value>) -> impl IntoResponse {
    let ctx = Ctx { state: app, sink: broadcast_sink(), host: Arc::new(WebHost), principal };
    match CommandRegistry::global().call(&cmd, &ctx, args).await {
        Ok(v)  => Json(ApiOk { data: v }).into_response(),
        Err(e) => e.into_response(),               // AppError → HTTP 状态码
    }
}
// 路由：POST /rpc/:cmd · GET /events(SSE) · GET /blob/:cmd · POST /auth/login · POST /auth/logout
```

### 4.3 前端收敛（业务组件零改，只动三个适配点）

| 关注点 | 收敛到 | Tauri 分支 | Web 分支 |
|--------|--------|-----------|----------|
| 请求 | `services/index.ts` 的 `ipc()` | `invoke(cmd,args)` | `fetch('/rpc/'+cmd,{method:'POST',body:JSON.stringify(args)})` |
| 事件 | 新增 `services` 的 `subscribe(cb)` | `listen('autoforge://event')` | `new EventSource('/events')` |
| 窗口 chrome | `src/lib/window.ts` + `isTauri` 判定 | 红绿灯/拖拽/自定义标题栏 | 隐藏 `.os-titlebar`，用浏览器边框 |

```ts
export const isTauri = '__TAURI_INTERNALS__' in window;
function ipc<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return isTauri ? invoke<T>(cmd, args)
                 : fetch(`/rpc/${cmd}`, {method:'POST', headers:{'Content-Type':'application/json'},
                         body: JSON.stringify(args ?? {})}).then(handleJson);
}
```
> 6 个页面里的 `listen('autoforge://event', …)` 全部改调 `subscribe(cb)`；`subscribe` 内部按 `isTauri` 分流。窗口按钮（`App.tsx`/`lib/window.ts` 的 `getCurrentWindow`）在 Web 下整条隐藏。

---

## 5. 多用户与权限（两模式同构）

**核心洞见**：桌面单机不是「没有用户」，而是「只有一个隐式 owner」。让 **`Principal` 永远流过 `Ctx`**，两模式即自动同时支持多用户——差异只在「principal 从哪来」。**authn（认证）在对接层，authz（授权）在 core**。

> 目标场景是**同一团队共享一个部署**（团队协作），不是隔离多个互不信任客户的多租户 SaaS。因此权限层要解决的是「团队内谁能做什么」（RBAC + 审计），不是「保证 A 客户看不到 B 客户的数据」（tenant isolation）——后者不在本设计范围内。

### 5.1 身份来源适配（对接层职责）

| 模式 | Principal 来源 |
|------|---------------|
| Tauri | `Principal::local_owner()`（全权）；可选：本地 PIN / OS 账户绑定后仍走同一 users 表 |
| Web | 登录（v1：用户名+密码即够用；团队已有 SSO/LDAP 时可选接 OIDC）→ session 或 JWT → 提取 `{ user_id, roles }` |

```rust
pub enum Principal {
    LocalOwner,                                  // 桌面默认：全权
    User { id: String, roles: Vec<Role> },       // Web / 桌面多用户
}
impl Principal { pub fn local_owner() -> Self { Principal::LocalOwner } }
```

### 5.2 数据模型（新增迁移 `0081_users_auth.sql`，不改旧表）

```sql
CREATE TABLE users (
  id TEXT PRIMARY KEY, username TEXT UNIQUE NOT NULL, display_name TEXT,
  password_hash TEXT,                 -- 密码登录用；OIDC 可空
  auth_provider TEXT NOT NULL DEFAULT 'local',
  status TEXT NOT NULL DEFAULT 'active', created_at TEXT NOT NULL);
CREATE TABLE roles (id TEXT PRIMARY KEY, name TEXT UNIQUE NOT NULL);
CREATE TABLE user_roles (user_id TEXT, role_id TEXT, PRIMARY KEY(user_id, role_id));
CREATE TABLE sessions (                -- 仅 Web 用；Tauri 不落
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
  token_hash TEXT NOT NULL, expires_at TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE user_project_roles (      -- 团队内项目级 ACL（如外部顾问只接触指定项目），非多租户隔离
  user_id TEXT, project_id TEXT, role TEXT, PRIMARY KEY(user_id, project_id));
```

### 5.3 权限点（RBAC，声明式）

- 全局角色：`owner / admin / operator / reviewer / viewer`。
- 关键动作绑权限点（写进 `#[command(perm="…")]`）：
  - `project.create` / `project.delete` / `project.write`
  - `cr.review_1` / **`cr.review_2`**（审批合并——**安全铁律，必须 gated**，对应现有唯一合并入口）
  - `settings.write` / `llm.key.write` / `agent.manage` / `intake.config`
  - `issue.write` / `conversation.write`
- 中央 guard：`dispatch` 在调 handler **前**统一 `authorize(&ctx.principal, def.perm)`，不过即拒（`AppError::Forbidden`）。
- 资源级细判（「只能审自己项目的 CR」）在 handler 内用 `ctx.principal` 补充。

```rust
fn authorize(p: &Principal, perm: Option<&str>) -> Result<(), AppError> {
    match (p, perm) {
        (Principal::LocalOwner, _) => Ok(()),                 // 桌面 owner 全通
        (_, None) => Ok(()),                                  // 无标注=公开命令
        (Principal::User{roles,..}, Some(pm)) =>
            if roles.iter().any(|r| r.grants(pm)) { Ok(()) } else { Err(AppError::Forbidden) },
    }
}
```

### 5.4 项目级可见性（团队 ACL，非多租户隔离）

现有查询全无 user 过滤。**在 core 数据访问处统一经 `scope(ctx)` 过滤器**，但目标场景是团队共享部署，默认应该是"全员可见团队内所有项目"：
- **默认**：`scope` = 恒等（全部可见），行为与今日桌面版完全一致——多数团队不需要更多。
- **可选收紧**：某些项目需要限定访问的团队成员（如含敏感信息的项目、外部顾问只接触自己负责的项目）时，`scope` 按 `user_project_roles` 收窄可见集。
- 统一走 `scope(ctx)` 这一个口子，是为了让"以后要不要加项目级限制"只改一处；**不是**为多租户 SaaS 预留隔离机制——那不是本设计的目标（见 §12）。

### 5.5 审计

`ctx.principal` 直接接入现有 `record_admin_decision*(… admin_id …)`（`change_requests.rs:1100` 等）——「谁批的合并/谁删的项目」自动留痕。`AppEvent` / 通知收件箱可加可选 `actor` 字段（`notification` 模型现无 actor，低优先补）。

### 5.6 密钥与启动配置（headless）

- 密钥保持**工厂级**（LLM/MCP key 全局，仅 `llm.key.write` gated）；MVP **不做 per-user 密钥库**，避免过早复杂化。
- 主密钥：`secrets.rs` 已支持 keyring→0600 文件回退，对接层启动时喂 `init_secrets(master_key_file)`：Tauri=`app_data_dir`，Web=配置/环境变量指定路径。
- 启动需注入的路径（替代 `app.path()`）：`app_data_dir`（→ `init_*_base`）、导出输出目录（替代 `download_dir`/`temp_dir`）。Web 头把「导出」改为 HTTP 下载后，输出目录仅用临时区。

---

## 6. 宿主能力抽象（收编 opener，其余无需抽象）

评估确认宿主专有能力**只有 opener** 真正在用（notification/shell 死注册）。

```rust
#[async_trait]
pub trait HostCapabilities: Send + Sync {
    /// 打开外部 URL。Tauri: opener 插件；Web: 返回指令让前端 window.open。
    async fn open_external(&self, url: &str) -> Result<(), AppError>;
    /// 在文件管理器中定位/打开本地路径。Web 无此语义 → 返回 Unsupported，调用方降级。
    async fn reveal_path(&self, path: &str) -> Result<(), AppError>;
}
```

- `TauriHost`：`open_external`→`app.opener().open_url`（`demo.rs:4`）；`reveal_path`→`reveal_item_in_dir`/`open_path`（`artifacts.rs:253`、`backup.rs:411`）。
- `WebHost`：`open_external`→回结构化结果 `{action:"open_external",url}` 交前端执行；`reveal_path`→`Err(Unsupported)`（前端隐藏「在文件夹中显示」按钮）。
- 现有 6 处 `OpenerExt` 直调统一改成 `ctx.host.*`。

---

## 7. 例外登记（两头行为必须不同的命令）

> 这些是「改一处」原则的**已知合法例外**——因宿主语义本质不同而必须在两层分别落地。数量极少，单独列册以防蔓延。

| 命令/能力 | Tauri 行为 | Web 行为 | 处理 |
|-----------|-----------|----------|------|
| `export_issues`（xlsx/csv，`issues.rs:451`） | 写入 `download_dir` 并可 reveal | `GET /blob/export_issues` 流式下载（`Content-Disposition`） | 迁到 `#[blob_command]`，core 产 `Blob{bytes,mime,filename}`，两层各自交付 |
| intake 导出（`intake.rs:312`） | 同上 | 同上 | 同上 |
| `reveal_path`（artifacts/backup） | 打开系统文件管理器定位 | 不支持 → 前端隐藏入口 | `HostCapabilities.reveal_path` 返回 `Unsupported` |
| `open_url`（`demo.rs`） | 系统浏览器打开 | 前端 `window.open` | `HostCapabilities.open_external` |
| 窗口控制（前端 `getCurrentWindow`） | 红绿灯/拖拽/最大化 | 隐藏，用浏览器边框 | `lib/window.ts` 按 `isTauri` 分支 |

---

## 8. 分阶段里程碑

> 本节按 §12 的战略拆分重新组织为两条**可独立立项**的轨道：**Track 1（M0–M3，无条件推进）**架构重构，价值不依赖 Web 头是否上线、全程零回归；**Track 2（M4 起）**团队内网协作，需先过一道轻量 Gate 再启动——但目标场景是团队内网协作而非公网 SaaS，门槛远低于"做完 M3 自然滑入 M4"字面听起来的重量级决策。

### Track 1 — 核心重构（现在做，无条件推进）

| 里程碑 | 内容 | 依赖 | 验收 |
|--------|------|------|------|
| **M0** R1 | `trait EventSink` + `TauriSink`；49 处 `AppHandle` 透传替换 | — | `cargo build` 通过；桌面端事件行为与今日一致（通知收件箱不丢） |
| **M1** R2+R6+workspace | `Ctx`/`#[command]` 宏/注册表；`HostCapabilities`；opener 收编；**顺手把 `crates/autoforge-core` 物理拆出**（§11，与其后补做不如一步到位） | M0 | 至少 1 个模块（如 `projects`）走注册表 dispatch，功能等价；`autoforge-core` 可独立 `cargo build` |
| **M2** R3 | 分批把 312 命令迁到统一契约（灰度回落旧路径）；**按功能域分批**，与该域的新功能开发错峰（§9 风险） | M1 | 每批迁移后 `tauri:dev` 全功能回归；`generate_handler!` 逐步清空 |
| **M3** R4 | Tauri 对接层瘦身为单 dispatch | M2 | 桌面端全功能等价；对接层不含命令清单；`cargo tree -p autoforge-core \| grep tauri` 为空 |

**先做且独立收益最大：M0**——即便永远不做 Web 头，它兑现了 CLAUDE.md「把 `AppHandle` 换成 `trait EventSink`」的既定愿景，减一层耦合、零回归、solo 维护者可独立完成。

### Gate — 启动 Track 2 前过一遍（决策点，比表面听起来轻量）

M3 完成后先回答这几条，而不是默认接着做 M4——但因为目标是**团队内网协作**、不是公网 SaaS，这道 Gate 不涉及融资/组建团队级别的决策，更多是"基础设施是否就绪"的落地问题（详细论证见 §12）：

1. 是否已有 ≥1 位真实同事/协作者明确表达过"希望不打开我的桌面客户端也能查看/操作 AutoForge"，而不是自己假设团队需要？
2. 团队内网是否有一台可以**常驻在线**的机器（办公室闲置主机、NAS、小型 Linux box）？—— 没有的话先解决这个前置问题，否则协作会因为"某人合上笔记本"而时断时续。
3. 是否已用**形态 A**（同进程双头、局域网访问，把当前 Tauri 内嵌的 Web 头直接开放给同事试用几天）低成本验证过协作需求？—— 没做过，先做这一步，成本几乎为零。
4. §5 的基本登录/会话/角色权限是否够用，还是团队已有 SSO/LDAP 需要对接？—— 内网场景通常前者就够，后者可作为 M5 之后的可选增强，不阻塞启动。

前三条有清楚答案就可以推进 Track 2；第 4 条不阻塞——因为不涉及对外网开放，鉴权模型可以边用边加固。

### Track 2 — 团队内网协作（条件推进，需求验证后启动）

| 里程碑 | 内容 | 依赖 | 验收 |
|--------|------|------|------|
| **M4** R5+R7 | Web 头（可继续嵌在 Tauri 进程内，也可编成独立 headless 二进制）+ 前端传输/事件/窗口自适应；部署到团队内网常驻机器，团队通过 `http://<内网地址>:port` 访问 | Gate 通过 + M3 | 团队 2+ 人通过内网浏览器同时访问，跑通只读页面 + 一条写路径 + SSE 事件；服务持续在线，不依赖某个人笔记本不合盖 |
| **M5** 团队协作角色 | 迁移 0081 + Principal 贯穿 + 声明式 authz；团队内按角色分工（谁能 `review_2` 批准合并、谁能删项目、谁能改 LLM key） | M4 | `cr.review_2` 需 reviewer 权限；越权请求被拒；**并发压测**（N 个账号同时提审/合并/发起会议室任务）不出竞态（见 §9 新增风险） |

> 明确移出路线图：面向不特定客户的**多租户公有 SaaS**（含计费、租户隔离、7×24 公网值守）不是 AutoForge 的目标——目标是团队内网协作。若未来真出现"给外部客户远程部署"的诉求，那是性质完全不同、需要单独立项评估的新项目，不应该现在为它预先设计（YAGNI）。

---

## 9. 风险与保障

| 风险 | 缓解 |
|------|------|
| 312 命令迁移量大、易漏 | M2 灰度回落旧路径，逐批迁移；每批跑 `tauri:dev` 回归；宏统一契约减少手写样板 |
| 参数命名 camelCase↔snake_case 不一致 | dispatch 侧做一次归一；迁移期用类型化入参结构由 serde 兜底 |
| M4→M5 过渡期：Web 头已在内网可达但登录/角色（M5）未落地，同网段任何人等效"全权 owner" | M4 阶段先只开放只读页面/团队内已知的少数写路径；`cr.review_2` 等敏感写路径优先跟 M5 一起上线，或过渡期用最简单的共享密码/token，不裸奔 |
| 团队 ACL 配错导致越权读（§5.4） | `scope(ctx)` 默认恒等、收紧是唯一改点；配授权测试（非多租户隔离场景，风险面本身就小） |
| `inventory`/`linkme` 跨平台/发布构建行为 | 早在 M1 于三端（Linux/macOS/Windows）发布构建各验一次收集是否完整 |
| 破坏「合并唯一入口」安全铁律 | `cr.review_2` 强制 `perm` 门；authz guard 单测覆盖「无权不能触发 merge」 |
| 团队服务器环境与桌面不一致（git/CLI 授权/工具链未装全） | M4 部署前对齐 checklist（对照桌面端依赖清单逐项装好），而非假设"跟本机一样" |
| 并发写路径此前只被单用户使用过，未经真实多用户压测 | M5 验收纳入并发压测（N 账号同时提审/合并/建会议室任务），验证既有锁（merge_lock/cr_lock/conversation_lock）在真实并发下不出竞态 |
| 内网可信 ≠ 零风险（同网段设备、访客 Wi-Fi 仍可能触达） | 即使内网也保留基本 session/token 鉴权（§5），不因为"内网"就完全裸奔 |
| 常驻服务器的运维责任（重启/备份/磁盘空间）集中在少数人身上 | 量级远小于 SaaS 运维；沿用现有「配置备份」功能的使用习惯即可，不需要新增组织能力 |
| 高迭代速度下 M2 与新功能并发抢 `commands/*.rs` | 按功能域分批迁移，迁移窗口内该域功能开发短暂错峰，而非全仓冻结 |
| 文档滞后误导 AI Agent（CLAUDE.md/specs 语境仍是单机假设） | Track 2 启动同时同步更新 CLAUDE.md/specs 的部署语境假设，纳入 M4 验收范围，防止「AI 照旧文档把 Web 头改回单机语义」的系统性跑偏 |

## 10. 验收标准（总）

1. **薄对接层红线**：新增一个业务命令，两个对接层 diff = 0（仅 core 改动）。
2. **桌面零回归**：M0–M3 每步 `npm run tauri:dev` 全功能等价。
3. **双头对等**：M4 后同一前端在 Tauri 与浏览器下核心只读/写路径 + 事件流均可用。
4. **权限有效**：M5 后 Web 未登录请求被拒；`cr.review_2` 等敏感动作按角色 gated；桌面 owner 行为无感。
5. **无 Tauri 泄漏**：core crate 依赖图不含 `tauri`（`cargo tree -p autoforge-core | grep tauri` 为空）。
6. **战略门禁**：M4 启动前必须有 §8 Gate 的书面答案，不得默认顺延（详见 §12）。

---

## 11. 代码组织：Workspace 拆分建议（而非仓库拆分）

回答"是否推荐拆分"里的**工程/仓库维度**。

**现状**：`src-tauri/Cargo.toml` 是单一 package，同时产出 `staticlib/cdylib/rlib`（供 Tauri）+ `bin`，未使用 Cargo workspace；`agents/core/tasks/models/db/state` 与 `commands/`（Tauri 命令层）混在同一 crate 内，靠人工审查/约定维持"业务不依赖 Tauri"（CLAUDE.md 已有此纪律，但纪律不是编译器）。

**建议：现在就拆 Cargo workspace（同仓库多 crate），不拆 git 仓库（不做 polyrepo）。**

拆 workspace 的理由：
- §10 验收标准第 5 条「`cargo tree -p autoforge-core | grep tauri` 为空」**必须有独立 crate 才能验证**——同一个 package 内"模块间不该互相依赖"没有编译期强制力，只能靠人工 review，长期会腐化（正是"薄对接层"这个诉求本身想避免的模式：约定会被悄悄破坏，边界要靠编译器守，不能只靠文档）。
- 与 R2（`#[command]` 宏/注册表）本就要抽 `Ctx` 同一批工作量，顺手把边界物理化，比"先在同 crate 里假装边界、以后再迁"更省一次返工。
- 对 solo 维护者友好：`cargo check -p autoforge-core` 可独立跑，不必每次全量编译 Tauri 壳，日常迭代反而更快。

不拆 git 仓库（不做 polyrepo）的理由：
- solo 维护 + 高迭代速度下（§1.4），polyrepo 会引入版本对齐、跨仓库 PR 协调、CI 矩阵翻倍等开销，而目前没有第二个团队或"独立发布 core"的真实驱动力去承担这些成本。
- Cargo workspace 的 crate 边界本身就是未来物理拆仓库的现成切割线——真到了「开源 core」或「Web 头交给独立团队」那天，直接把 `crates/autoforge-core` 平移出去即可；现在拆 polyrepo 反而是提前支付一笔用不上的税。

**建议布局**（并入 M1，见 §8）：
```
AutoForge/                    (仍是单一 git 仓库)
  Cargo.toml                  (新增：workspace 声明)
  crates/
    autoforge-core/           (agents/ core/ tasks/ models/ db/ state/ 从 src-tauri/src 平移)
    autoforge-macros/         (#[command] proc-macro，见 §3.2)
  src-tauri/                  (瘦身：仅 dispatch + capabilities + tauri.conf.json，依赖 autoforge-core)
  web/                        (Track 2 才新增：axum 头，同样只依赖 autoforge-core)
```

---

## 12. 战略决策：是否推荐拆分（长期价值判断）

综合 §1.4 的组织现实评估，分三层给出明确建议：

**(a) 架构拆分（core vs 对接层，即 Track 1）—— 推荐，无条件推进。**
即使 Web 头永远不做，这也是 CLAUDE.md 早已确立的长期愿景（"后端独立化"）的落地：改善可测试性、消灭 `AppHandle` 透传耦合，且完全向后兼容、零业务风险，solo 维护者可独立完成，不需要新增任何组织能力。**这部分不是"要不要做"的问题，是"什么时候顺手做"的问题。**

**(b) 代码/仓库拆分（workspace vs polyrepo）—— 推荐现在拆 workspace，不推荐拆仓库。** 见 §11，理由不重复。

**(c) Web 头 + 团队内网协作（Track 2）—— 目标已明确为团队协作而非 SaaS，门槛显著低于最初评估，可以相对从容地推进，但仍建议先过轻量 Gate 再投入，避免为想象中的团队规模过度设计。**

这个判断建立在一个关键澄清上：**AutoForge 的 Web 头要解决的是"团队内网协作"，不是"面向不特定客户的多租户 SaaS"**。这个澄清直接消解了此前评估里最重的两块顾虑：

1. **"本地能力远程化"不再是难题**——此前评估把它当作最大障碍，是因为设想了"每个客户的环境都不一样、要在我方服务器上逐一重建"（多租户场景）。团队内网协作只有**一套环境**：一台团队共享的常驻机器，装好 git/`claude` CLI 授权/编译工具链，这和"一个人的笔记本"在架构上是同一件事——只是从"一个 GUI 窗口"变成"多个浏览器连同一个后端"。真正要做的是 M4（部署到内网机器）+ M5（团队角色协作），不需要"重建 N 份隔离环境"那种量级的工程。
2. **组织能力缺口大幅缩小**——公有 SaaS 需要的合规、计费、多租户隔离验证、7×24 公网值守，团队内网场景**全部不需要**。剩下的运维责任只是"服务器别宕机、有人记得备份"，1 人团队完全扛得住，不需要等"融资/组建团队"。

因此 Track 2 不再需要长期搁置——§8 的 Gate 已相应改写为轻量版（有没有常驻机器、有没有真实协作诉求、验证过没有、鉴权模型够不够用），都是可以在几天内低成本回答的问题，不是重量级商业决策。

**具体行动建议：**
1. **现在**：批准并执行 Track 1（M0–M3 + workspace 拆分）。纯正收益、可独立交付、无需新增组织能力。
2. **Track 1 完成后**：过一遍 §8 的轻量 Gate——核心是"内网有没有一台常驻机器"和"真的有同事需要非桌面访问"这两件事，而不是"要不要做 SaaS"这种重决策。验证过就可以直接推进 M4 → M5，不需要无限期搁置。
3. **保持在路线图之外**：多租户公有 SaaS（对外向不特定客户提供托管服务、需要计费与租户隔离）不是当前产品方向的自然延伸。若未来真出现这类诉求，那是一个需要从零单独评估的新项目，不应该现在为它预先设计（YAGNI）。

一句话：**架构层面的"拆"（core/对接层/workspace）现在就该做，代价几乎为零；Track 2 一旦明确是"团队内网协作"而非"SaaS"，也不再是需要长期观望的重决策——过一遍轻量 Gate（有没有常驻机器、有没有真实协作诉求）就可以推进，真正应该排除在路线图之外的只有"面向外部客户的多租户 SaaS"这一项。**

---

## 附录：本文引用的关键证据

- 事件出口与副作用：`src-tauri/src/core/event.rs:166`、`:172`
- AppState 纯字段：`src-tauri/src/state.rs:17`
- 密钥回退：`src-tauri/src/core/secrets.rs:44`、`:113`、`:168`
- 审批 admin_id：`src-tauri/src/commands/change_requests.rs:1100`
- opener 用途：`demo.rs:4`、`artifacts.rs:253/258`、`backup.rs:411/416`
- app.path 用途：`lib.rs:36`、`issues.rs:452`、`intake.rs:313`
- 导出落盘：`issues.rs:299/451`、`intake.rs:312`
- 前端单一入口：`src/services/index.ts:6`（唯一 invoke 出口）
- listen 散点：`App.tsx` / `Dashboard.tsx` / `Projects.tsx` / `Settings.tsx` / `Conversations.tsx` / `Audit.tsx`
- 迁移基线：`src-tauri/migrations/0080_batch_bind_source.sql`（下一号 0081）
- 维护者规模：`git shortlog -sn --all`（`Renmengkai` 150 次人类提交；`AutoForge`/`autoforge@local` 39 次，经 `git log --author=AutoForge --format="%ae"` 核实为自动化 CR 流水线提交，非第二人类维护者）；提交时间分布：`git log --format="%ad" --date=format:"%Y-%m"`（186 次集中于 2026-05~06）
</content>
</invoke>
