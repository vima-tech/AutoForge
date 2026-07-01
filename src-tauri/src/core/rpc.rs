//! 统一命令契约（双头架构 M1 种子）——DUAL_HEAD.md §3、§5。
//!
//! 目标：让业务命令写「强类型舒适签名」`async fn(ctx: &Ctx, args) -> Result<Ret, RpcError>`，
//! 由注册表做「类型擦除」的统一 dispatch，使 Tauri 壳与未来 Web 头共用同一套命令——
//! 加/删业务命令时两个对接层 diff = 0（对接层不枚举命令名，只做「认身份·转请求·桥事件·供宿主」）。
//!
//! 本模块是**可编译+可单测的种子**：Ctx / Principal / authorize / CommandRegistry / dispatch
//! 全部落地并测通；与旧 `#[tauri::command]` **并存**（DUAL_HEAD §3.3 渐进迁移），不动现有命令。
//! 把命令真正迁进注册表 + 在 lib.rs 挂 Tauri dispatch + 迁 295 处 State——属 M2/M3（需 GUI 回归），另做。
//!
//! 铁律：本模块纯 Rust，不引用 `tauri::*`；事件只经 `Ctx.sink`（EventSink 抽象）。

use crate::core::event::EventSink;
use crate::state::AppState;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// 调用者身份（DUAL_HEAD §5.1）。桌面壳默认 `LocalOwner`（全权）；Web 头登录后为 `User`。
/// authn（认证）在对接层解析出 Principal，authz（授权）在 core 统一判定——两模式同构。
#[derive(Clone, Debug)]
pub enum Principal {
    /// 桌面单机的隐式 owner：全权。
    LocalOwner,
    /// 已认证用户（Web 头 / 桌面多用户）。
    User { id: String, roles: Vec<Role> },
}

impl Principal {
    pub fn local_owner() -> Self {
        Principal::LocalOwner
    }
}

/// 全局角色（DUAL_HEAD §5.3）。RBAC 声明式权限点由 [`Role::grants`] 判定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Operator,
    Reviewer,
    Viewer,
}

impl Role {
    /// 该角色是否被授予某权限点（如 `cr.review_2` / `project.delete`）。
    /// 采用「前缀域 + 角色能力」的保守映射：owner/admin 全通；operator 管执行与配置；
    /// reviewer 管审核（含合并唯一入口 `cr.review_2`——安全铁律，必须 gated）；viewer 只读。
    pub fn grants(&self, perm: &str) -> bool {
        match self {
            Role::Owner | Role::Admin => true,
            Role::Operator => {
                // 执行/发起类 + 需求/会话写；不含删项目、改密钥、批准合并。
                perm.starts_with("issue.")
                    || perm.starts_with("conversation.")
                    || perm.starts_with("cr.review_1")
                    || perm == "project.write"
                    || perm.starts_with("intake.")
            }
            Role::Reviewer => {
                // 审核者：两个审核节点（含合并唯一入口）。
                perm.starts_with("cr.review_1") || perm.starts_with("cr.review_2")
            }
            Role::Viewer => false, // 只读：不授予任何写权限点
        }
    }
}

/// 命令执行的统一上下文（取代到处透传的 `AppHandle` + `State`）。
///
/// 廉价可 `Clone`（`state`/`sink` 为 `Arc`，`principal` 为小枚举）——handler 按值收 `Ctx`，
/// 返回 `'static` future，避免 handler 注册表的借用生命周期纠缠。
#[derive(Clone)]
pub struct Ctx {
    pub state: Arc<AppState>,
    pub sink: Arc<dyn EventSink>,
    pub principal: Principal,
}

impl Ctx {
    pub fn new(state: Arc<AppState>, sink: Arc<dyn EventSink>, principal: Principal) -> Self {
        Self {
            state,
            sink,
            principal,
        }
    }
    /// 便捷取 DB（多数命令只需要它）。
    pub fn db(&self) -> &crate::db::Db {
        &self.state.db
    }
}

/// 统一错误 → 对接层各自映射（Tauri: `String`；Web: HTTP 状态码）。
#[derive(Debug)]
pub enum RpcError {
    /// 无此命令（注册表未命中）。
    NotFound(String),
    /// 入参反序列化失败。
    BadArgs(String),
    /// 授权不通过（越权）。
    Forbidden,
    /// 业务执行失败。
    Internal(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::NotFound(c) => write!(f, "unknown command: {c}"),
            RpcError::BadArgs(e) => write!(f, "bad arguments: {e}"),
            RpcError::Forbidden => write!(f, "forbidden"),
            RpcError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RpcError {}

impl From<anyhow::Error> for RpcError {
    fn from(e: anyhow::Error) -> Self {
        RpcError::Internal(e.to_string())
    }
}

/// 声明式授权点（DUAL_HEAD §5.3）：dispatch 在调 handler **前**统一判定，不过即拒。
/// `perm=None`（命令未标注）= 公开命令；`LocalOwner` 全通。
pub fn authorize(p: &Principal, perm: Option<&str>) -> Result<(), RpcError> {
    match (p, perm) {
        (Principal::LocalOwner, _) => Ok(()),
        (_, None) => Ok(()),
        (Principal::User { roles, .. }, Some(pm)) => {
            if roles.iter().any(|r| r.grants(pm)) {
                Ok(())
            } else {
                Err(RpcError::Forbidden)
            }
        }
    }
}

/// 类型擦除的 handler：按值收 `Ctx` + JSON 入参，返回 `'static` future。
pub type Handler = Arc<
    dyn Fn(Ctx, serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value, RpcError>>
        + Send
        + Sync,
>;

/// 一条命令的注册项。
pub struct CommandDef {
    pub name: &'static str,
    /// 声明式授权点；`None` = 公开命令。
    pub perm: Option<&'static str>,
    pub handler: Handler,
}

/// 命令注册表：name → 定义。构建一次，dispatch 时 O(1) 查表。
#[derive(Default)]
pub struct CommandRegistry {
    map: HashMap<&'static str, CommandDef>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条**强类型**业务命令：宏擦除入参/返回的 serde 编解码，业务函数写舒适签名。
    /// `f: Fn(Ctx, A) -> Future<Output = Result<R, RpcError>>`。
    pub fn register<A, R, F, Fut>(
        &mut self,
        name: &'static str,
        perm: Option<&'static str>,
        f: F,
    ) where
        A: DeserializeOwned + Send + 'static,
        R: Serialize + 'static,
        F: Fn(Ctx, A) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, RpcError>> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |ctx, args_json| {
            // 同步解析入参 → 调业务 fn 得 future（f 经 &self 调用，不移入 async 块）→ 擦除返回。
            match serde_json::from_value::<A>(args_json) {
                Ok(a) => {
                    let fut = f(ctx, a);
                    Box::pin(async move {
                        let r = fut.await?;
                        serde_json::to_value(r).map_err(|e| RpcError::Internal(e.to_string()))
                    })
                }
                Err(e) => Box::pin(async move { Err(RpcError::BadArgs(e.to_string())) }),
            }
        });
        self.map.insert(name, CommandDef { name, perm, handler });
    }

    pub fn get(&self, name: &str) -> Option<&CommandDef> {
        self.map.get(name)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 统一分发：查表 → 授权 → 调 handler。对接层只调此一处，不枚举命令名（DUAL_HEAD 红线）。
    pub async fn dispatch(
        &self,
        cmd: &str,
        ctx: Ctx,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let def = self.get(cmd).ok_or_else(|| RpcError::NotFound(cmd.to_string()))?;
        authorize(&ctx.principal, def.perm)?;
        (def.handler)(ctx, args).await
    }
}

/// 把上下文基质的只读命令登记进注册表——**首批走统一契约的真实业务命令**样板，
/// 证明真实命令（非合成 echo）能套进 `Ctx` 契约。与 `commands/context.rs` 的旧
/// `#[tauri::command]` 并存（DUAL_HEAD §3.3）；M2 收口时把 Tauri dispatch 指向注册表后，
/// 旧包装即可退役。此函数编译通过本身即验证「真实命令的强类型签名能被擦除登记」。
pub fn register_substrate_commands(reg: &mut CommandRegistry) {
    use crate::core::context;

    #[derive(serde::Deserialize)]
    struct ListArgs {
        project_id: String,
        kinds: Option<Vec<String>>,
        limit: Option<i64>,
    }
    reg.register("ctx.list", None, |ctx: Ctx, a: ListArgs| async move {
        let kinds = a.kinds.unwrap_or_default();
        let kr: Vec<&str> = kinds.iter().map(|s| s.as_str()).collect();
        context::list(ctx.db(), &a.project_id, &kr, a.limit.unwrap_or(200))
            .await
            .map_err(RpcError::from)
    });

    #[derive(serde::Deserialize)]
    struct AssembleArgs {
        project_id: String,
        include: Option<Vec<String>>,
        refs: Option<Vec<String>>,
        budget_bytes: Option<i64>,
    }
    reg.register("ctx.assemble", None, |ctx: Ctx, a: AssembleArgs| async move {
        let req = context::ContextRequest {
            project_id: a.project_id,
            include: a.include.unwrap_or_default(),
            refs: a.refs.unwrap_or_default(),
            budget_bytes: a.budget_bytes.unwrap_or(0),
        };
        context::assemble(ctx.db(), &req).await.map_err(RpcError::from)
    });

    #[derive(serde::Deserialize)]
    struct FetchArgs {
        id: String,
        max_chars: Option<i64>,
    }
    reg.register("ctx.fetch", None, |ctx: Ctx, a: FetchArgs| async move {
        let item = context::get(ctx.db(), &a.id)
            .await
            .map_err(RpcError::from)?
            .ok_or_else(|| RpcError::NotFound(a.id.clone()))?;
        context::fetch_content(ctx.db(), &item, a.max_chars.unwrap_or(8192) as usize)
            .await
            .map_err(RpcError::from)
    });
}

/// 进程级命令注册表（构建一次）。当前登记走统一契约的首批命令（基质只读命令）；
/// M2 逐域迁移时把更多命令加进 `register_*` 系列，Tauri/Web 两个对接层的 dispatch 都查此表。
pub fn global_registry() -> &'static CommandRegistry {
    static REG: std::sync::OnceLock<CommandRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(|| {
        let mut r = CommandRegistry::new();
        register_substrate_commands(&mut r);
        r
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // —— 授权判定（纯逻辑，无需 Ctx）——

    #[test]
    fn local_owner_passes_everything() {
        assert!(authorize(&Principal::LocalOwner, Some("cr.review_2")).is_ok());
        assert!(authorize(&Principal::LocalOwner, Some("project.delete")).is_ok());
        assert!(authorize(&Principal::LocalOwner, None).is_ok());
    }

    #[test]
    fn unmarked_command_is_public() {
        let viewer = Principal::User { id: "u".into(), roles: vec![Role::Viewer] };
        assert!(authorize(&viewer, None).is_ok(), "无 perm 标注 = 公开命令");
    }

    #[test]
    fn reviewer_gates_merge_but_viewer_denied() {
        let reviewer = Principal::User { id: "r".into(), roles: vec![Role::Reviewer] };
        let viewer = Principal::User { id: "v".into(), roles: vec![Role::Viewer] };
        // 合并唯一入口 cr.review_2 是安全铁律：reviewer 可、viewer 拒。
        assert!(authorize(&reviewer, Some("cr.review_2")).is_ok());
        assert!(matches!(
            authorize(&viewer, Some("cr.review_2")),
            Err(RpcError::Forbidden)
        ));
        // viewer 连删项目也不行。
        assert!(matches!(
            authorize(&viewer, Some("project.delete")),
            Err(RpcError::Forbidden)
        ));
    }

    #[test]
    fn operator_can_issue_not_review2() {
        let op = Principal::User { id: "o".into(), roles: vec![Role::Operator] };
        assert!(authorize(&op, Some("issue.write")).is_ok());
        assert!(matches!(
            authorize(&op, Some("cr.review_2")),
            Err(RpcError::Forbidden)
        ), "operator 不得批准合并");
    }

    // —— 注册表 dispatch（端到端：注册 → 擦除编解码 → 授权 → 调用）——

    fn mk_ctx(db: crate::db::Db, principal: Principal) -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let state = AppState {
            db,
            job_tx: tx,
            concurrency: crate::core::concurrency::ConcurrencyManager::new(5, 20),
            dev_servers: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            webhook_handle: Arc::new(tokio::sync::Mutex::new(None)),
            asr_sessions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            autosupply_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        Ctx {
            state: Arc::new(state),
            sink: Arc::new(NoopSink),
            principal,
        }
    }

    fn test_ctx(principal: Principal) -> Ctx {
        // dispatch/authz 逻辑不碰具体字段，惰性内存库即可构造。
        mk_ctx(crate::db::Db::connect_lazy("sqlite::memory:").unwrap(), principal)
    }

    struct NoopSink;
    impl EventSink for NoopSink {
        fn emit(&self, _event: crate::core::event::AppEvent) {}
    }

    #[derive(Deserialize)]
    struct AddArgs {
        a: i64,
        b: i64,
    }

    #[tokio::test]
    async fn dispatch_roundtrips_typed_command() {
        let mut reg = CommandRegistry::new();
        reg.register("add", None, |_ctx: Ctx, args: AddArgs| async move {
            Ok::<i64, RpcError>(args.a + args.b)
        });
        let out = reg
            .dispatch("add", test_ctx(Principal::LocalOwner), serde_json::json!({"a": 2, "b": 3}))
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!(5));
    }

    #[tokio::test]
    async fn dispatch_unknown_command_is_not_found() {
        let reg = CommandRegistry::new();
        let err = reg
            .dispatch("nope", test_ctx(Principal::LocalOwner), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)));
    }

    #[tokio::test]
    async fn dispatch_enforces_authz_before_handler() {
        let mut reg = CommandRegistry::new();
        // 标注 cr.review_2 权限点的命令：viewer 应在进 handler 前被拒。
        reg.register("approve_merge", Some("cr.review_2"), |_ctx: Ctx, _args: serde_json::Value| async move {
            Ok::<bool, RpcError>(true)
        });
        let viewer = Principal::User { id: "v".into(), roles: vec![Role::Viewer] };
        let err = reg
            .dispatch("approve_merge", test_ctx(viewer), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Forbidden));
        // owner 同一命令通过。
        let ok = reg
            .dispatch("approve_merge", test_ctx(Principal::LocalOwner), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(ok, serde_json::json!(true));
    }

    /// 端到端：真实基质命令 `ctx.list` 经 dispatch 查真库 → 序列化返回。
    #[tokio::test]
    async fn dispatch_real_substrate_command_end_to_end() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE context_index (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                origin_stage TEXT NOT NULL DEFAULT '', origin_actor TEXT NOT NULL DEFAULT '',
                content_ref TEXT NOT NULL DEFAULT '', size_hint INTEGER NOT NULL DEFAULT 0,
                trust TEXT NOT NULL DEFAULT 'trusted', labels TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&db)
        .await
        .unwrap();
        crate::core::context::register(
            &db,
            crate::core::context::NewContextItem::trusted(
                "p1",
                crate::core::context::source_kind::ISSUE,
                "i1",
                "需求",
                "issue:i1",
            ),
        )
        .await
        .unwrap();

        let ctx = mk_ctx(db, Principal::LocalOwner);
        let mut reg = CommandRegistry::new();
        register_substrate_commands(&mut reg);

        let out = reg
            .dispatch("ctx.list", ctx, serde_json::json!({"project_id": "p1"}))
            .await
            .unwrap();
        let arr = out.as_array().expect("返回 JSON 数组");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["source_id"], "i1");
        assert_eq!(arr[0]["id"], "issue:i1");
    }

    /// 全局注册表（rpc_dispatch 查此表）已登记基质只批命令——验证 Tauri 接线。
    #[test]
    fn global_registry_has_substrate_commands() {
        let reg = global_registry();
        assert!(reg.get("ctx.list").is_some());
        assert!(reg.get("ctx.assemble").is_some());
        assert!(reg.get("ctx.fetch").is_some());
        assert!(reg.len() >= 3);
    }

    #[tokio::test]
    async fn dispatch_bad_args_surface_as_bad_args() {
        let mut reg = CommandRegistry::new();
        reg.register("add", None, |_ctx: Ctx, args: AddArgs| async move {
            Ok::<i64, RpcError>(args.a + args.b)
        });
        let err = reg
            .dispatch("add", test_ctx(Principal::LocalOwner), serde_json::json!({"a": "x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::BadArgs(_)));
    }
}
