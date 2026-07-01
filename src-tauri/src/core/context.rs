//! 上下文基质 L2（数据侧）——方法论平台/基质设计 §3、§4.1。
//!
//! 把 L1 既有各表投影为统一的 [`ContextItem`]（薄索引，不搬正文），提供三种访问里的
//! 前两种地基：**枚举/检索**（[`list`]，只回元数据）与**稳定引用登记**（[`register`]）。
//! 正文懒取（access #2）与装配引擎（L3）在后续批次（B1/B2）叠加。
//!
//! 铁律（对齐 CLAUDE.md 后端独立化愿景）：本模块**纯 Rust**，不引用 `tauri::*`；
//! 外部来源（trust=external_untrusted）回灌上下文前必过 `security::has_obvious_injection`。

use crate::db::Db;
use crate::models::context_item::ContextItem;
use anyhow::Result;

/// 来源类型全集（基质设计 §3.2）。自由字符串，此处仅登记「约定值」便于统一引用；
/// 新增来源 = 加一个常量 + 一个投影适配器，不动既有子系统。
pub mod source_kind {
    // —— 静态文件类（已在旧上下文装配路径内） ——
    pub const FILE_PRIORITY: &str = "file_priority"; // claude.md / agents.md
    pub const FILE_PINNED: &str = "file_pinned"; // conversation_project_context
    pub const WORKSPACE_DOC: &str = "workspace_doc"; // .autoforge/docs
    pub const WORKSPACE_SPEC: &str = "workspace_spec"; // .autoforge/specs
    pub const WORKSPACE_DELIVERABLE: &str = "workspace_deliverable"; // .autoforge/deliverables
    pub const PROJECT_SPEC: &str = "project_spec"; // project_specs

    // —— 物料 + 过程信息（当前游离在上下文之外的关键缺口，§2.2） ——
    pub const MATERIAL: &str = "material"; // material_files
    pub const CHAT_MESSAGE: &str = "chat_message"; // messages
    pub const AGENT_OUTPUT: &str = "agent_output"; // conversation_task_runs
    pub const CODE_AGENT_LOG: &str = "code_agent_log"; // code_agent_run_logs
    pub const LLM_TRACE: &str = "llm_trace"; // llm_traces
    pub const ISSUE: &str = "issue"; // issues
    pub const CR_REVIEW: &str = "cr_review"; // change_requests + admin_decisions
    pub const INCUBATOR_DRAFT: &str = "incubator_draft"; // blueprint_*
    pub const ATTACHMENT: &str = "attachment"; // conversation_attachments

    // —— 全量覆盖新增来源（万物可引 · 实施契约 §5） ——
    pub const SECURITY_AUDIT: &str = "security_audit"; // security_audits
    pub const TEST_SESSION: &str = "test_session"; // test_sessions
    pub const SCAN_FINDING: &str = "scan_finding"; // scan_findings
    pub const DEPLOYMENT: &str = "deployment"; // deployments
    pub const DELIVERY_ARTIFACT: &str = "delivery_artifact"; // delivery_artifacts
    pub const WORKTREE_SESSION: &str = "worktree_session"; // worktree_sessions
    pub const PROTOTYPE_PROMPT: &str = "prototype_prompt"; // prototype_prompts
    pub const PROJECT_META: &str = "project_meta"; // .autoforge/claude.md · agents.md
    pub const CFG_AGENT: &str = "cfg_agent"; // agents（脱敏）
    pub const CFG_CODE_AGENT: &str = "cfg_code_agent"; // code_agents（脱敏）
    pub const CFG_MCP: &str = "cfg_mcp"; // mcp_servers（脱敏，仅非密文字段）

    // —— 外部不可信来源（trust=external_untrusted，必过注入闸） ——
    pub const MCP_RESULT: &str = "mcp_result";
    pub const WEB_RESULT: &str = "web_result";
}

/// 信任级别（基质设计 §3.3）。内部产生=trusted；外部来源=external_untrusted，
/// 回灌上下文前必过 `has_obvious_injection`。
pub mod trust {
    pub const TRUSTED: &str = "trusted";
    pub const EXTERNAL_UNTRUSTED: &str = "external_untrusted";
}

/// 稳定引用：由 `source_kind` + `source_id` 派生，保证同一来源条目只对应一个 id。
pub fn stable_id(source_kind: &str, source_id: &str) -> String {
    format!("{source_kind}:{source_id}")
}

/// 一条待登记的上下文条目（借用字段，避免不必要拷贝）。
pub struct NewContextItem<'a> {
    pub project_id: &'a str,
    pub source_kind: &'a str,
    pub source_id: &'a str,
    pub title: &'a str,
    pub origin_stage: &'a str,
    pub origin_actor: &'a str,
    pub content_ref: &'a str,
    pub size_hint: i64,
    pub trust: &'a str,
    /// 自由标签 JSON 数组字符串（如 `["prd","design"]`）；空则传 `"[]"`。
    pub labels: &'a str,
}

impl<'a> NewContextItem<'a> {
    /// 最常见形态：内部可信来源，无阶段/标签的快捷构造。
    pub fn trusted(
        project_id: &'a str,
        source_kind: &'a str,
        source_id: &'a str,
        title: &'a str,
        content_ref: &'a str,
    ) -> Self {
        Self {
            project_id,
            source_kind,
            source_id,
            title,
            origin_stage: "",
            origin_actor: "",
            content_ref,
            size_hint: 0,
            trust: trust::TRUSTED,
            labels: "[]",
        }
    }
}

/// 幂等登记一条上下文条目到薄索引。同一来源（source_kind+source_id）再次登记时
/// **刷新**元数据（title/content_ref/size_hint/labels/updated_at），不产生重复行。
/// 正文不入库——只登记「有这么一条、在哪取、多大、可不可信」。
pub async fn register(db: &Db, item: NewContextItem<'_>) -> Result<()> {
    let id = stable_id(item.source_kind, item.source_id);
    sqlx::query(
        "INSERT INTO context_index
           (id, project_id, source_kind, source_id, title, origin_stage, origin_actor,
            content_ref, size_hint, trust, labels, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
         ON CONFLICT(id) DO UPDATE SET
            title=excluded.title,
            origin_stage=excluded.origin_stage,
            origin_actor=excluded.origin_actor,
            content_ref=excluded.content_ref,
            size_hint=excluded.size_hint,
            trust=excluded.trust,
            labels=excluded.labels,
            updated_at=datetime('now')",
    )
    .bind(&id)
    .bind(item.project_id)
    .bind(item.source_kind)
    .bind(item.source_id)
    .bind(item.title)
    .bind(item.origin_stage)
    .bind(item.origin_actor)
    .bind(item.content_ref)
    .bind(item.size_hint)
    .bind(item.trust)
    .bind(item.labels)
    .execute(db)
    .await?;
    Ok(())
}

/// 移除一条已登记条目（原始来源被删时同步清理索引）。
pub async fn deregister(db: &Db, source_kind: &str, source_id: &str) -> Result<()> {
    let id = stable_id(source_kind, source_id);
    sqlx::query("DELETE FROM context_index WHERE id=?")
        .bind(&id)
        .execute(db)
        .await?;
    Ok(())
}

/// 枚举一个项目的候选上下文条目（只回元数据，不回正文；access #1）。
/// `kinds` 为空则不按来源类型过滤；结果按产生时间倒序，`limit` 为返回总上限。
///
/// **pull 模型**（《全量上下文基质·万物可引》契约 §3）：从数据活查（provider 层枚举全量来源），
/// 而非只读 `context_index`。`context_index` 仅作可选缓存（现由 register 钩子预热，此处不依赖）。
/// 总预算 `limit` 摊到各命中来源，避免单一大表淹没其余来源。
pub async fn list(
    db: &Db,
    project_id: &str,
    kinds: &[&str],
    limit: i64,
) -> Result<Vec<ContextItem>> {
    use crate::core::context_providers as cp;
    let repo = project_repo_path(db, project_id).await;
    let n_sources = if kinds.is_empty() {
        (cp::SOURCES.len() + cp::FILE_SOURCES.len()) as i64
    } else {
        kinds.len() as i64
    };
    // 每来源软上限：总预算摊平，但至少 10、至多 limit（单来源筛选时给满）。
    let per_source = (limit / n_sources.max(1)).clamp(10, limit.max(10));
    let mut items = cp::enumerate_all(db, project_id, kinds, repo.as_deref(), per_source).await?;

    // 叠加 context_index 缓存（register 钩子预热 / 历史条目）：provider 未覆盖的按 id 补入，
    // provider 结果优先（活查是真源，缓存仅兜底/加速）。
    let seen: std::collections::HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
    if let Ok(cached) = list_cached(db, project_id, kinds, limit).await {
        for c in cached {
            if !seen.contains(&c.id) {
                items.push(c);
            }
        }
    }
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    items.truncate(limit.max(0) as usize);
    Ok(items)
}

/// 只读 `context_index` 缓存（旧 push 路径的查询；现作 [`list`] 的缓存叠加层）。
async fn list_cached(
    db: &Db,
    project_id: &str,
    kinds: &[&str],
    limit: i64,
) -> Result<Vec<ContextItem>> {
    let mut sql = String::from("SELECT * FROM context_index WHERE project_id=?");
    if !kinds.is_empty() {
        sql.push_str(" AND source_kind IN (");
        sql.push_str(&vec!["?"; kinds.len()].join(","));
        sql.push(')');
    }
    sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, ContextItem>(&sql).bind(project_id);
    for k in kinds {
        q = q.bind(*k);
    }
    q = q.bind(limit);
    Ok(q.fetch_all(db).await?)
}

/// 解析项目本地仓库路径（文件来源 provider 需要；无仓库则回 None）。
pub(crate) async fn project_repo_path(db: &Db, project_id: &str) -> Option<String> {
    sqlx::query_as::<_, (Option<String>,)>("SELECT repo_path FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .and_then(|(v,)| v)
        .filter(|s| !s.is_empty())
}

/// 取单条上下文条目元数据（供显式 ref 定位）。
///
/// pull 模型下：先查 `context_index` 缓存（若被 register 预热）；缓存 miss 时从稳定 id
/// （`<kind>:<source_id>`）**反构最小条目**——只要 kind 是已知 provider 来源即可，
/// 后续 `fetch_content` 按 kind+source_id 懒取正文。这样显式 ref 无需依赖缓存即可解析。
pub async fn get(db: &Db, id: &str) -> Result<Option<ContextItem>> {
    if let Some(item) = sqlx::query_as::<_, ContextItem>("SELECT * FROM context_index WHERE id=?")
        .bind(id)
        .fetch_optional(db)
        .await?
    {
        return Ok(Some(item));
    }
    // 缓存 miss：从 id 反构（kind:source_id）。kind 不含 ':'，故 split_once 正确。
    if let Some((kind, source_id)) = id.split_once(':') {
        let known = crate::core::context_providers::providers()
            .iter()
            .any(|p| p.kind() == kind);
        if known {
            return Ok(Some(ContextItem {
                id: id.to_string(),
                project_id: String::new(),
                source_kind: kind.to_string(),
                source_id: source_id.to_string(),
                title: String::new(),
                preview: String::new(),
                origin_stage: String::new(),
                origin_actor: String::new(),
                content_ref: format!("lazy:{kind}:{source_id}"),
                size_hint: 0,
                trust: trust::TRUSTED.to_string(),
                labels: "[]".to_string(),
                created_at: String::new(),
                updated_at: String::new(),
            }));
        }
    }
    Ok(None)
}

/// 一次上下文装配请求（基质设计 §4.2 的动作侧输入）。
#[derive(Debug, Clone, Default)]
pub struct ContextRequest {
    pub project_id: String,
    /// 要纳入的 source_kind 集合（空 = 不限来源，取全部）。
    pub include: Vec<String>,
    /// 显式点名的 ContextItem.id（用户/上一环节指定）——最高优先，永远纳入。
    pub refs: Vec<String>,
    /// 体积预算（字节）。0 = 不限（仅按排序返回全部候选）。
    pub budget_bytes: i64,
}

/// 来源类型的装配优先级（越小越优先），对应 §4.2「refs > 优先文件 > 规格 > 近期过程信息 > 其余」。
/// refs 单独处理（永远置顶），不走此表。
fn kind_rank(kind: &str) -> u8 {
    use source_kind as k;
    match kind {
        k::FILE_PRIORITY | k::FILE_PINNED => 0,
        k::PROJECT_SPEC | k::WORKSPACE_SPEC => 1,
        k::WORKSPACE_DOC | k::WORKSPACE_DELIVERABLE => 2,
        k::ISSUE | k::CR_REVIEW | k::INCUBATOR_DRAFT => 3,
        // 近期过程信息
        k::CODE_AGENT_LOG | k::LLM_TRACE | k::AGENT_OUTPUT | k::CHAT_MESSAGE => 4,
        k::MATERIAL | k::ATTACHMENT => 5,
        // 外部不可信来源垫底
        k::MCP_RESULT | k::WEB_RESULT => 6,
        _ => 5,
    }
}

/// 选材：把候选按「显式 refs 置顶 → kind 优先级 → 近期（输入已按时间倒序）」排序，
/// 再按体积预算裁剪。**纯函数**，不碰 DB/IO，便于单测装配策略本身（§4.2 步骤 2+4）。
///
/// - refs（显式点名）永远纳入，不受预算裁剪（用户明确要的必须给）。
/// - 其余候选按排序累加 size_hint，超预算即停（budget_bytes=0 表示不限）。
/// - 稳定排序：同 kind 内保留输入的时间倒序。
pub fn select(candidates: Vec<ContextItem>, req: &ContextRequest) -> Vec<ContextItem> {
    let ref_set: std::collections::HashSet<&str> = req.refs.iter().map(|s| s.as_str()).collect();

    // 1) 拆成 refs 命中 / 其余。
    let (mut refs_hit, mut rest): (Vec<ContextItem>, Vec<ContextItem>) = candidates
        .into_iter()
        .partition(|c| ref_set.contains(c.id.as_str()));

    // refs 按 req.refs 给定顺序排列（用户意图顺序）。
    refs_hit.sort_by_key(|c| {
        req.refs
            .iter()
            .position(|r| r == &c.id)
            .unwrap_or(usize::MAX)
    });

    // 2) 其余按 kind 优先级稳定排序（同 kind 保留输入的时间倒序）。
    rest.sort_by_key(|c| kind_rank(&c.source_kind));

    // 3) 预算裁剪：refs 全纳入并预扣其体积；其余在剩余预算内累加。
    let mut out: Vec<ContextItem> = Vec::with_capacity(refs_hit.len() + rest.len());
    let mut used: i64 = 0;
    for it in refs_hit {
        used += it.size_hint.max(0);
        out.push(it);
    }
    for it in rest {
        let sz = it.size_hint.max(0);
        if req.budget_bytes > 0 && used + sz > req.budget_bytes {
            continue; // 超预算的低优先候选跳过（不 break：小体积的后续项仍可能塞进剩余预算）
        }
        used += sz;
        out.push(it);
    }
    out
}

/// 装配：查候选（按项目 + include 来源）+ 补齐显式 refs，再走 [`select`] 定序裁剪。
/// 返回**入选条目的元数据有序列表**；正文渲染（懒取 + 截断 + 拼 prompt）在 B2 叠加。
pub async fn assemble(db: &Db, req: &ContextRequest) -> Result<Vec<ContextItem>> {
    let kinds: Vec<&str> = req.include.iter().map(|s| s.as_str()).collect();
    let mut candidates = list(db, &req.project_id, &kinds, 500).await?;

    // 补齐不在 include 候选里的显式 refs（可能是别的来源类型）。
    let have: std::collections::HashSet<String> =
        candidates.iter().map(|c| c.id.clone()).collect();
    for id in &req.refs {
        if !have.contains(id) {
            if let Some(item) = get(db, id).await? {
                candidates.push(item);
            }
        }
    }
    Ok(select(candidates, req))
}

/// 大体量过程信息来源：懒取时只投影「尾部摘要」而非全文（G3，防日志/trace 撑爆预算）。
/// 单一真源：读 `SOURCES` 声明的 `bulky` 标志（新增大体量来源只需在声明里 `.bulky()`）。
fn is_bulky(kind: &str) -> bool {
    crate::core::context_providers::SOURCES
        .iter()
        .any(|s| s.kind == kind && s.bulky)
}

/// UTF-8 安全的保尾截断（大日志/trace 的尾部通常是结论/最新状态/改动摘要，比保头有用）。
fn keep_tail(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let skip = n - max_chars;
    format!("…{}", s.chars().skip(skip).collect::<String>())
}

/// 生成一个 `context_ref` 消息块的 JSON（G4 三处同步的 Rust 侧）：让 Agent 消息可引用一条
/// 基质条目，前端 `Block.tsx` 的 `context_ref` 分支渲染成可展开卡片（点开走 `fetch_context_content`）。
/// 字段与 `src/data/mock.ts` 的 `context_ref` 块类型一致：`{ t, ref, kind, title }`。
pub fn context_ref_block(item: &ContextItem) -> serde_json::Value {
    serde_json::json!({
        "t": "context_ref",
        "ref": item.id,
        "kind": item.source_kind,
        "title": item.title,
    })
}

/// 跑一条「返回单个可空 TEXT 列 + 单 id bind」的静态查询；取不到/为 NULL 回空串。
async fn fetch_col(db: &Db, sql: &str, id: &str) -> Result<String> {
    Ok(sqlx::query_as::<_, (Option<String>,)>(sql)
        .bind(id)
        .fetch_optional(db)
        .await?
        .and_then(|(v,)| v)
        .unwrap_or_default())
}

/// 懒取一条上下文条目的正文（基质 §4.1 access #2）。按 `content_ref` 的 scheme 分派到
/// **静态读取器**（scheme 白名单，避免动态表名 SQL）；大体量来源投影尾部摘要（G3）。
/// `max_chars` 为返回字符上限。正文永远此刻从原表/文件取，索引不缓存正文。
///
/// content_ref 约定：`file:<path>` / `issue:<id>` / `<未知或空>`→回退标题。
/// 新增来源 = 在此加一条 scheme 分支（+ 在写入侧 register 时用对应 scheme 拼 content_ref）。
pub async fn fetch_content(db: &Db, item: &ContextItem, max_chars: usize) -> Result<String> {
    // scheme 白名单 → 静态 SQL（表/列写死，绝不动态拼表名；id 走 bind 防注入）。
    // 新增来源 = 加一条分支（写入侧 register 用对应 scheme 拼 content_ref）。
    let raw = match item.content_ref.split_once(':') {
        Some(("file", path)) => tokio::fs::read_to_string(path).await.unwrap_or_default(),
        Some(("issue", id)) => {
            fetch_col(db, "SELECT description FROM issues WHERE id=?", id).await?
        }
        // 编码 Agent 执行日志（大体量 → is_bulky 保尾摘要，G3）。
        Some(("clog", id)) => {
            fetch_col(db, "SELECT stdout FROM code_agent_run_logs WHERE id=?", id).await?
        }
        // 会议室消息（content_json 原样，交由渲染侧解析块）。
        Some(("msg", id)) => {
            fetch_col(db, "SELECT content_json FROM messages WHERE id=?", id).await?
        }
        // Agent 历史输出（大体量 → 保尾摘要）。
        Some(("atr", id)) => {
            fetch_col(db, "SELECT output_text FROM conversation_task_runs WHERE id=?", id).await?
        }
        // 孵化台草稿：取当前 PRD（Markdown）作为正文。
        Some(("bp", id)) => {
            fetch_col(db, "SELECT prd_markdown FROM blueprint_drafts WHERE id=?", id).await?
        }
        // CR 审核意见：拼接该 CR 两个审核节点人工填写的全部非空建议（「人在审核里说过什么」）。
        Some(("crv", cr_id)) => {
            fetch_col(
                db,
                "SELECT group_concat(suggestions, char(10)) FROM admin_decisions
                 WHERE change_request_id=? AND suggestions IS NOT NULL AND suggestions != ''",
                cr_id,
            )
            .await?
        }
        // pull 模型统一出口：无旧 scheme（含 `lazy:<kind>:<id>` 与空）→ 按 source_kind + source_id
        // 派发到 provider 层懒取（覆盖全量新增来源）；provider 未命中再用标题兜底。
        _ => {
            let repo = if item.project_id.is_empty() {
                None
            } else {
                project_repo_path(db, &item.project_id).await
            };
            // provider 取正文失败/空一律回落标题：缺失或坏掉的单个来源不应让懒取报错。
            crate::core::context_providers::fetch_kind(
                db,
                &item.source_kind,
                &item.source_id,
                repo.as_deref(),
                max_chars,
            )
            .await
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| item.title.clone())
        }
    };
    Ok(if is_bulky(&item.source_kind) {
        keep_tail(&raw, max_chars)
    } else {
        crate::core::security::safe_truncate(&raw, max_chars)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // 与迁移 0081 同构的最小建表（单测不跑完整迁移链）。
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
        .execute(&p)
        .await
        .unwrap();
        p
    }

    /// 登记是幂等的：同一来源二次登记刷新元数据、不产生重复行。
    #[tokio::test]
    async fn register_is_idempotent_and_refreshes() {
        let db = pool().await;
        let mut v1 =
            NewContextItem::trusted("proj1", source_kind::CODE_AGENT_LOG, "cr-42", "执行日志 v1", "lazy:code_agent_log:cr-42");
        v1.size_hint = 100;
        register(&db, v1).await.unwrap();
        let mut v2 =
            NewContextItem::trusted("proj1", source_kind::CODE_AGENT_LOG, "cr-42", "执行日志 v2", "lazy:code_agent_log:cr-42");
        v2.size_hint = 250;
        register(&db, v2).await.unwrap();

        let rows = list(&db, "proj1", &[], 10).await.unwrap();
        assert_eq!(rows.len(), 1, "同一来源只登记一行");
        assert_eq!(rows[0].title, "执行日志 v2", "二次登记刷新标题");
        assert_eq!(rows[0].size_hint, 250, "二次登记刷新体积");
        assert_eq!(rows[0].id, "code_agent_log:cr-42", "稳定 id = kind:source_id");
    }

    /// 枚举按项目隔离 + 按来源类型过滤 + 时间倒序。
    #[tokio::test]
    async fn list_filters_by_project_and_kind() {
        let db = pool().await;
        register(&db, NewContextItem::trusted("p1", source_kind::MATERIAL, "m1", "物料1", "file:a.md"))
            .await
            .unwrap();
        register(&db, NewContextItem::trusted("p1", source_kind::LLM_TRACE, "t1", "trace1", "lazy:llm_trace:t1"))
            .await
            .unwrap();
        register(&db, NewContextItem::trusted("p2", source_kind::MATERIAL, "m2", "物料2", "file:b.md"))
            .await
            .unwrap();

        // 项目隔离：p1 只见自己的两条。
        assert_eq!(list(&db, "p1", &[], 10).await.unwrap().len(), 2);
        assert_eq!(list(&db, "p2", &[], 10).await.unwrap().len(), 1);
        // 来源过滤：p1 仅物料 → 1 条。
        let mats = list(&db, "p1", &[source_kind::MATERIAL], 10).await.unwrap();
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].source_id, "m1");
    }

    /// 构造用于测 `select`（纯函数）的合成条目。
    fn item(id: &str, kind: &str, size: i64) -> ContextItem {
        ContextItem {
            id: id.to_string(),
            project_id: "p".to_string(),
            source_kind: kind.to_string(),
            source_id: id.to_string(),
            title: id.to_string(),
            preview: String::new(),
            origin_stage: String::new(),
            origin_actor: String::new(),
            content_ref: String::new(),
            size_hint: size,
            trust: trust::TRUSTED.to_string(),
            labels: "[]".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// 选材排序：显式 refs 置顶（按 req.refs 顺序），其余按 kind 优先级。
    #[test]
    fn select_puts_refs_first_then_kind_priority() {
        let candidates = vec![
            item("mat1", source_kind::MATERIAL, 10),        // rank 5
            item("prio", source_kind::FILE_PRIORITY, 10),   // rank 0
            item("log1", source_kind::CODE_AGENT_LOG, 10),  // rank 4
            item("spec1", source_kind::PROJECT_SPEC, 10),   // rank 1
        ];
        let req = ContextRequest {
            project_id: "p".into(),
            refs: vec!["log1".into()], // 显式点名日志 → 置顶，压过优先文件
            budget_bytes: 0,
            ..Default::default()
        };
        let out: Vec<String> = select(candidates, &req).into_iter().map(|c| c.id).collect();
        assert_eq!(out, vec!["log1", "prio", "spec1", "mat1"], "refs 置顶，其余按 kind 优先级");
    }

    /// 预算裁剪：refs 永远纳入（即便预算紧），低优先超预算候选被跳过。
    #[test]
    fn select_respects_budget_but_always_keeps_refs() {
        let candidates = vec![
            item("prio", source_kind::FILE_PRIORITY, 100),
            item("big", source_kind::MATERIAL, 1000),
            item("small", source_kind::MATERIAL, 50),
            item("refd", source_kind::WEB_RESULT, 30), // 低优先但被显式 ref → 强纳，预扣 30
        ];
        let req = ContextRequest {
            project_id: "p".into(),
            refs: vec!["refd".into()],
            budget_bytes: 200, // refd(30) 预扣后剩 170：prio(100)+small(50)=150 塞得下，big(1000)塞不下
            ..Default::default()
        };
        let out: Vec<String> = select(candidates, &req).into_iter().map(|c| c.id).collect();
        assert!(out.contains(&"refd".to_string()), "ref 强纳（低优先也进）");
        assert!(out.contains(&"prio".to_string()), "高优先小体积纳入");
        assert!(out.contains(&"small".to_string()), "预算内小体积纳入（跳过 big 后仍可塞）");
        assert!(!out.contains(&"big".to_string()), "超预算的大体积被跳过");
    }

    /// assemble：查库候选 + 补齐跨来源显式 ref + 定序。
    #[tokio::test]
    async fn assemble_queries_and_orders() {
        let db = pool().await;
        register(&db, NewContextItem::trusted("p1", source_kind::MATERIAL, "m1", "物料", "file:a.md"))
            .await
            .unwrap();
        register(&db, NewContextItem::trusted("p1", source_kind::FILE_PRIORITY, "claude", "claude.md", "file:claude.md"))
            .await
            .unwrap();
        // 另一来源类型，不在 include 里，但被显式 ref → assemble 应补齐。
        register(&db, NewContextItem::trusted("p1", source_kind::LLM_TRACE, "t1", "trace", "lazy:llm_trace:t1"))
            .await
            .unwrap();

        let req = ContextRequest {
            project_id: "p1".into(),
            include: vec![source_kind::MATERIAL.into(), source_kind::FILE_PRIORITY.into()],
            refs: vec!["llm_trace:t1".into()], // 跨 include 的显式 ref
            budget_bytes: 0,
            ..Default::default()
        };
        let out: Vec<String> = assemble(&db, &req).await.unwrap().into_iter().map(|c| c.id).collect();
        // 显式 ref 置顶，其余按 kind：file_priority(0) < material(5)。
        assert_eq!(out, vec!["llm_trace:t1", "file_priority:claude", "material:m1"]);
    }

    /// 保尾截断（G3 摘要投影）：UTF-8 安全，超长取尾部并加省略号。
    #[test]
    fn keep_tail_is_utf8_safe() {
        assert_eq!(keep_tail("短", 10), "短");
        // 10 个中文字，取尾 3 → "…" + 末 3 字。
        let s = "零一二三四五六七八九";
        assert_eq!(keep_tail(s, 3), "…七八九");
    }

    /// 懒取正文：file scheme 读文件，issue scheme 读 issues.description，
    /// 大体量来源（code_agent_log）走保尾摘要。
    #[tokio::test]
    async fn fetch_content_dispatches_by_scheme() {
        let db = pool().await;
        sqlx::query("CREATE TABLE issues (id TEXT PRIMARY KEY, description TEXT)")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO issues VALUES ('i1', '这是需求正文描述')")
            .execute(&db)
            .await
            .unwrap();

        // issue scheme
        let issue_item = ContextItem {
            content_ref: "issue:i1".into(),
            ..item("issue:i1", source_kind::ISSUE, 0)
        };
        assert_eq!(fetch_content(&db, &issue_item, 100).await.unwrap(), "这是需求正文描述");

        // file scheme（临时文件）
        let tmp = std::env::temp_dir().join(format!("ctx_fetch_{}.txt", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, "文件正文内容").await.unwrap();
        let file_item = ContextItem {
            content_ref: format!("file:{}", tmp.display()),
            ..item("f1", source_kind::WORKSPACE_DOC, 0)
        };
        assert_eq!(fetch_content(&db, &file_item, 100).await.unwrap(), "文件正文内容");
        let _ = tokio::fs::remove_file(&tmp).await;

        // 大体量来源保尾：把长文放临时文件，code_agent_log 走 keep_tail。
        let tmp2 = std::env::temp_dir().join(format!("ctx_log_{}.txt", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp2, "零一二三四五六七八九").await.unwrap();
        let log_item = ContextItem {
            content_ref: format!("file:{}", tmp2.display()),
            ..item("l1", source_kind::CODE_AGENT_LOG, 0)
        };
        assert_eq!(fetch_content(&db, &log_item, 3).await.unwrap(), "…七八九", "大体量来源保尾摘要");
        let _ = tokio::fs::remove_file(&tmp2).await;
    }

    /// clog scheme：从 code_agent_run_logs.stdout 读，且大体量走保尾摘要。
    #[tokio::test]
    async fn fetch_content_reads_clog_with_tail_summary() {
        let db = pool().await;
        sqlx::query("CREATE TABLE code_agent_run_logs (id TEXT PRIMARY KEY, stdout TEXT NOT NULL DEFAULT '')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO code_agent_run_logs (id, stdout) VALUES ('r1', '零一二三四五六七八九')")
            .execute(&db)
            .await
            .unwrap();
        let clog_item = ContextItem {
            content_ref: "clog:r1".into(),
            ..item("r1", source_kind::CODE_AGENT_LOG, 0)
        };
        // code_agent_log 是 bulky → 保尾 3 字。
        assert_eq!(fetch_content(&db, &clog_item, 3).await.unwrap(), "…七八九");
        // 未命中的 id → 空串（不 panic）。
        let missing = ContextItem {
            content_ref: "clog:nope".into(),
            ..item("nope", source_kind::CODE_AGENT_LOG, 0)
        };
        assert_eq!(fetch_content(&db, &missing, 100).await.unwrap(), "");
    }

    /// bp scheme：从 blueprint_drafts.prd_markdown 读孵化台草稿正文。
    #[tokio::test]
    async fn fetch_content_reads_incubator_draft() {
        let db = pool().await;
        sqlx::query("CREATE TABLE blueprint_drafts (id TEXT PRIMARY KEY, prd_markdown TEXT NOT NULL DEFAULT '')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO blueprint_drafts (id, prd_markdown) VALUES ('d1', '# PRD\n大需求内容')")
            .execute(&db)
            .await
            .unwrap();
        let it = ContextItem {
            content_ref: "bp:d1".into(),
            ..item("d1", source_kind::INCUBATOR_DRAFT, 0)
        };
        assert_eq!(fetch_content(&db, &it, 100).await.unwrap(), "# PRD\n大需求内容");
    }

    /// crv scheme：拼接某 CR 两个审核节点的非空建议（跳过空建议）。
    #[tokio::test]
    async fn fetch_content_reads_cr_review_suggestions() {
        let db = pool().await;
        sqlx::query(
            "CREATE TABLE admin_decisions (id TEXT PRIMARY KEY, change_request_id TEXT, suggestions TEXT)",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO admin_decisions (id, change_request_id, suggestions) VALUES
               ('d1','cr1','需求审核：范围收窄到登录页'),
               ('d2','cr1',''),
               ('d3','cr1','代码审核：补一个空密码用例')",
        )
        .execute(&db)
        .await
        .unwrap();
        let it = ContextItem {
            content_ref: "crv:cr1".into(),
            ..item("cr1", source_kind::CR_REVIEW, 0)
        };
        let out = fetch_content(&db, &it, 200).await.unwrap();
        assert!(out.contains("范围收窄到登录页"));
        assert!(out.contains("补一个空密码用例"));
        assert!(!out.contains("\n\n\n"), "空建议被跳过，不留空行堆叠");
    }

    /// context_ref 块 JSON 形状与 TS 类型（mock.ts）一致：{ t, ref, kind, title }。
    #[test]
    fn context_ref_block_shape_matches_ts() {
        let it = item("code_agent_log:cr1", source_kind::CODE_AGENT_LOG, 0);
        let b = context_ref_block(&it);
        assert_eq!(b["t"], "context_ref");
        assert_eq!(b["ref"], "code_agent_log:cr1");
        assert_eq!(b["kind"], source_kind::CODE_AGENT_LOG);
        assert!(b.get("title").is_some());
    }

    /// deregister 清理索引（原始来源被删时同步）。
    #[tokio::test]
    async fn deregister_removes_row() {
        let db = pool().await;
        register(&db, NewContextItem::trusted("p1", source_kind::ISSUE, "i1", "需求1", "table:issues.description#i1"))
            .await
            .unwrap();
        assert_eq!(list(&db, "p1", &[], 10).await.unwrap().len(), 1);
        deregister(&db, source_kind::ISSUE, "i1").await.unwrap();
        assert_eq!(list(&db, "p1", &[], 10).await.unwrap().len(), 0);
    }
}
