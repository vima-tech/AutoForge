//! 全量上下文基质 · pull-provider 层（《全量上下文基质·万物可引》实施契约 §3、§4）。
//!
//! 架构翻转：把"每个子系统写数据时主动 `register` 推进索引"（push）翻成"从数据活查"（pull）。
//! 加一种来源 = 往静态 [`SOURCES`] / [`FILE_SOURCES`] 加一行声明，**一处**（不变量 I1）。
//! `context_index` 表由此降级为可选缓存；本模块是"活查"的唯一实现。
//!
//! 铁律（对齐后端独立化愿景）：**纯 Rust**，不引用 `tauri::*`。
//! 安全（I3/I4）：黑名单表 [`NEVER_CONTEXT`] 绝不建 provider；正文列显式点名 → 密钥列天然不被取；
//! 外部来源 `fetch` 回灌前过 `has_obvious_injection`。

use crate::core::context::{source_kind as sk, stable_id, trust};
use crate::db::Db;
use crate::models::context_item::ContextItem;
use anyhow::Result;
use async_trait::async_trait;

/// 密钥 / 基础设施 / 自指表：**绝不可**作为上下文来源（不变量 I3）。
/// 这些表**不出现在 [`SOURCES`] 里**；[`tests::sources_never_touch_blacklist`] 断言零交集。
pub const NEVER_CONTEXT: &[&str] = &[
    // 密钥（即使已信封加密也不可作为可浏览上下文）
    "llm_configs",
    "widget_tokens",
    "intake_configs",
    "notify_channels",
    // 基础设施 / 自指 / 无正文
    "app_settings",
    "job_executions",
    "context_index",
    "conversation_reads",
    "_sqlx_migrations",
];

/// 一条声明式的 DB 表来源。所有标识符类字段是**编译期字面量**（开发者维护，永不承载运行时/外部输入）；
/// 运行时数据（project_id / source_id）一律走 bind。见契约 §3.2 的注入边界说明。
pub struct TableSource {
    pub kind: &'static str,
    pub table: &'static str,
    /// 主键列（→ source_id）。默认多为 `id`。
    pub id_col: &'static str,
    /// 生成标题的 SQL 表达式（可含别名 `t.`，如 `t.title` / `substr(t.content_json,1,80)`）。
    pub title_sql: &'static str,
    /// 生成预览片段的 SQL 表达式；`""` = 默认按 `substr(t.<content_col>,1,200)`。
    /// JSON 正文（如会议室 content_json）可覆盖为 `json_extract(...)` 取可读文本。
    pub preview_sql: &'static str,
    /// 懒取正文的列（显式点名 → 密钥列天然不被取）。
    pub content_col: &'static str,
    /// 排序时间列。
    pub time_col: &'static str,
    pub stage: &'static str,
    pub bulky: bool,
    /// FROM + JOIN 片段；主表别名恒为 `t`（如 `issues t` 或
    /// `messages t JOIN conversations c ON c.id=t.conversation_id`）。
    pub scope_from: &'static str,
    /// 项目列限定（如 `t.project_id` / `c.project_id`）；`""` = 全局来源（不按项目过滤）。
    pub scope_project: &'static str,
    /// 额外恒定谓词（静态字面量，如 `t.parent_id IS NULL` 只取 root span）；`""` = 无。
    pub extra_where: &'static str,
    pub trust: &'static str,
}

impl TableSource {
    const fn new(
        kind: &'static str,
        table: &'static str,
        title_sql: &'static str,
        content_col: &'static str,
        scope_from: &'static str,
        scope_project: &'static str,
    ) -> Self {
        TableSource {
            kind,
            table,
            id_col: "id",
            title_sql,
            preview_sql: "",
            content_col,
            time_col: "created_at",
            stage: "",
            bulky: false,
            scope_from,
            scope_project,
            extra_where: "",
            trust: trust::TRUSTED,
        }
    }
    const fn stage(mut self, s: &'static str) -> Self {
        self.stage = s;
        self
    }
    const fn bulky(mut self) -> Self {
        self.bulky = true;
        self
    }
    const fn time(mut self, c: &'static str) -> Self {
        self.time_col = c;
        self
    }
    const fn id(mut self, c: &'static str) -> Self {
        self.id_col = c;
        self
    }
    const fn extra(mut self, w: &'static str) -> Self {
        self.extra_where = w;
        self
    }
    const fn preview(mut self, s: &'static str) -> Self {
        self.preview_sql = s;
        self
    }
}

/// 全量 DB 来源清单（契约 §5）。加一张表 = 加一行。
/// **黑名单表不得出现在此**（单测强制）。
pub static SOURCES: &[TableSource] = &[
    // —— 需求线 ——
    TableSource::new(
        sk::ISSUE,
        "issues",
        "t.title",
        "description",
        "issues t",
        "t.project_id",
    )
    .stage("requirement"),
    // —— 孵化台 ——
    TableSource::new(
        sk::INCUBATOR_DRAFT,
        "blueprint_drafts",
        "t.title",
        "prd_markdown",
        "blueprint_drafts t",
        "t.project_id",
    )
    .stage("requirement"),
    // —— 项目规格 ——
    TableSource::new(
        sk::PROJECT_SPEC,
        "project_specs",
        "t.title",
        "content",
        "project_specs t",
        "t.project_id",
    )
    .stage("design"),
    // —— 编码执行日志（大体量保尾）；经 worktree_sessions→change_requests 拿 project_id ——
    TableSource::new(
        sk::CODE_AGENT_LOG,
        "code_agent_run_logs",
        "'编码日志 · ' || substr(t.id,1,8)",
        "stdout",
        "code_agent_run_logs t",
        "",
    )
    .stage("coding")
    .bulky()
    .time("created_at"),
    // —— LLM trace（只取 root span=一次 Agent 调用）；直连 project_id，大体量保尾 ——
    TableSource::new(
        sk::LLM_TRACE,
        "llm_traces",
        "COALESCE(t.name,'trace') || ' · ' || COALESCE(t.agent_name,'')",
        "output",
        "llm_traces t",
        "t.project_id",
    )
    .stage("coding")
    .bulky()
    .time("created_at")
    .extra("t.parent_id IS NULL"),
    // —— 会议室原始消息；经 conversations 一跳拿 project_id，排除软删会话 ——
    TableSource::new(
        sk::CHAT_MESSAGE,
        "messages",
        // content_json=[{"t":"md","md":"…"}]（quote_ref 用 text、code 用 code）：取首块可读文本，
        // 逐个键回落，最后回落原始 JSON 截断。
        "COALESCE(NULLIF(json_extract(t.content_json,'$[0].md'),''), \
                  NULLIF(json_extract(t.content_json,'$[0].text'),''), \
                  NULLIF(json_extract(t.content_json,'$[0].code'),''), \
                  substr(t.content_json,1,80))",
        "content_json",
        "messages t JOIN conversations c ON c.id = t.conversation_id",
        "c.project_id",
    )
    .stage("chat")
    .preview(
        "COALESCE(NULLIF(json_extract(t.content_json,'$[0].md'),''), \
                  NULLIF(json_extract(t.content_json,'$[0].text'),''), \
                  NULLIF(json_extract(t.content_json,'$[0].code'),''), \
                  substr(t.content_json,1,200))",
    )
    .extra("c.deleted_at IS NULL"),
    // —— Agent 任务输出；经 conversation_tasks→conversations 两跳，大体量保尾 ——
    TableSource::new(
        sk::AGENT_OUTPUT,
        "conversation_task_runs",
        "'Agent 输出 · ' || COALESCE(t.agent_id,'')",
        "output_text",
        "conversation_task_runs t \
         JOIN conversation_tasks k ON k.id = t.task_id \
         JOIN conversations c ON c.id = k.conversation_id",
        "c.project_id",
    )
    .stage("chat")
    .bulky()
    .time("started_at"),
    // —— CR 审核意见（人在审核里说过什么）；直连 project_id ——
    TableSource::new(
        sk::CR_REVIEW,
        "admin_decisions",
        "COALESCE(t.stage,'审核') || ' · ' || COALESCE(t.decision,'')",
        "suggestions",
        "admin_decisions t",
        "t.project_id",
    )
    .stage("review")
    .extra("t.suggestions IS NOT NULL AND t.suggestions != ''"),
    // —— 安全审计 ——
    TableSource::new(
        sk::SECURITY_AUDIT,
        "security_audits",
        "'安全审计 · ' || COALESCE(t.status,'')",
        "summary",
        "security_audits t",
        "t.project_id",
    )
    .stage("review")
    .time("started_at"),
    // —— 测试会话 ——
    TableSource::new(
        sk::TEST_SESSION,
        "test_sessions",
        "'测试 · ' || COALESCE(t.session_type,'') || ' · ' || COALESCE(t.status,'')",
        "summary",
        "test_sessions t",
        "t.project_id",
    )
    .stage("review")
    .time("started_at"),
    // —— 扫描发现；经 test_sessions 一跳拿 project_id ——
    TableSource::new(
        sk::SCAN_FINDING,
        "scan_findings",
        "COALESCE(t.severity,'') || ' · ' || COALESCE(t.title,'')",
        "description",
        "scan_findings t JOIN test_sessions s ON s.id = t.test_session_id",
        "s.project_id",
    )
    .stage("review"),
    // —— 部署（大体量日志保尾）——
    TableSource::new(
        sk::DEPLOYMENT,
        "deployments",
        "'部署 · ' || COALESCE(t.target_env,'') || ' · ' || COALESCE(t.status,'')",
        "log",
        "deployments t",
        "t.project_id",
    )
    .stage("ops")
    .bulky(),
    // —— 交付产物 ——
    TableSource::new(
        sk::DELIVERY_ARTIFACT,
        "delivery_artifacts",
        "COALESCE(t.original_name,'交付产物')",
        "description",
        "delivery_artifacts t",
        "t.project_id",
    )
    .stage("ops"),
    // —— worktree 会话（报告正文）；经 change_requests 一跳 ——
    TableSource::new(
        sk::WORKTREE_SESSION,
        "worktree_sessions",
        "'worktree · ' || COALESCE(t.branch_name,'')",
        "report_content",
        "worktree_sessions t JOIN change_requests r ON r.id = t.change_request_id",
        "r.project_id",
    )
    .stage("coding")
    .time("started_at"),
    // —— 原型提示词 ——
    TableSource::new(
        sk::PROTOTYPE_PROMPT,
        "prototype_prompts",
        "COALESCE(t.title,'原型提示')",
        "prompt",
        "prototype_prompts t",
        "t.project_id",
    )
    .stage("design"),
    // —— 物料库（正文=元数据描述；文件深读二期）——
    TableSource::new(
        sk::MATERIAL,
        "material_files",
        "COALESCE(t.original_name,'物料')",
        "description",
        "material_files t",
        "t.project_id",
    )
    .stage("design"),
    // —— 会议室附件（正文=文件名；经 conversations 一跳）——
    TableSource::new(
        sk::ATTACHMENT,
        "conversation_attachments",
        "COALESCE(t.original_name,'附件')",
        "original_name",
        "conversation_attachments t JOIN conversations c ON c.id = t.conversation_id",
        "c.project_id",
    )
    .stage("chat"),
    // —— 配置类（全局，字段级脱敏：绝不点名密文列）——
    TableSource::new(
        sk::CFG_AGENT,
        "agents",
        "'Agent · ' || COALESCE(t.name,'')",
        "system_prompt",
        "agents t",
        "",
    )
    .stage("design"),
    TableSource::new(
        sk::CFG_CODE_AGENT,
        "code_agents",
        "'编码Agent · ' || COALESCE(t.label,'')",
        "model",
        "code_agents t",
        "",
    )
    .stage("coding"),
    // mcp_servers：只点名非密文列（command）；env_json/headers_json 绝不出现。
    TableSource::new(
        sk::CFG_MCP,
        "mcp_servers",
        "'MCP · ' || COALESCE(t.name,'') || ' · ' || COALESCE(t.transport,'')",
        "command",
        "mcp_servers t",
        "",
    )
    .stage("ops"),
];

/// 一条声明式的 `.autoforge` 文件来源（文件系统，非 DB）。
pub struct FileSource {
    pub kind: &'static str,
    /// `.autoforge/` 下的子目录（递归 walk）；与 `files` 二选一。
    pub subdir: Option<&'static str>,
    /// `.autoforge/` 下点名的具体文件（如 claude.md）；与 `subdir` 二选一。
    pub files: &'static [&'static str],
    pub stage: &'static str,
}

/// `.autoforge` 文件来源清单（契约 §5）。
pub static FILE_SOURCES: &[FileSource] = &[
    FileSource {
        kind: sk::WORKSPACE_DOC,
        subdir: Some("docs"),
        files: &[],
        stage: "design",
    },
    FileSource {
        kind: sk::WORKSPACE_SPEC,
        subdir: Some("specs"),
        files: &[],
        stage: "design",
    },
    FileSource {
        kind: sk::WORKSPACE_DELIVERABLE,
        subdir: Some("deliverables"),
        files: &[],
        stage: "ops",
    },
    FileSource {
        kind: sk::PROJECT_META,
        subdir: None,
        files: &["claude.md", "agents.md"],
        stage: "design",
    },
];

// ── SourceProvider 抽象 ─────────────────────────────────────────────────────

#[async_trait]
pub trait SourceProvider: Send + Sync {
    fn kind(&self) -> &'static str;
    fn trust(&self) -> &'static str {
        trust::TRUSTED
    }
    /// 活查该项目下本来源的条目元数据（不搬正文）。
    /// `query` 有值时按标题子串过滤（DB 来源下推 LIKE；文件来源按相对路径过滤）。
    async fn enumerate(
        &self,
        db: &Db,
        project_id: &str,
        repo_path: Option<&str>,
        limit: i64,
        query: Option<&str>,
    ) -> Result<Vec<ContextItem>>;
    /// 懒取一条正文。外部来源实现须自行过注入闸（本层在 [`fetch_kind`] 统一兜一道）。
    async fn fetch(
        &self,
        db: &Db,
        source_id: &str,
        repo_path: Option<&str>,
        max_chars: usize,
    ) -> Result<String>;
}

/// LIKE 模式转义：`%` `_` `\` 前加 `\` 再包 `%…%`（配合 `ESCAPE '\'`），
/// 防用户输入被当通配符。中文子串是字节级匹配，天然可用。
pub(crate) fn like_pattern(q: &str) -> String {
    let mut esc = String::with_capacity(q.len() + 8);
    for c in q.chars() {
        if c == '%' || c == '_' || c == '\\' {
            esc.push('\\');
        }
        esc.push(c);
    }
    format!("%{esc}%")
}

/// 标识符安全校验（防 typo 字面量逃逸成注入）：仅允许 `[a-z_][a-z0-9_.]*`。
/// 注意：`title_sql` / `scope_from` / `extra_where` 是受信任的自由 SQL 片段（非用户输入），不走此校验。
fn is_safe_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .map(|c| c.is_ascii_lowercase() || c == '_')
            .unwrap_or(false)
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

/// DB 表来源的通用 provider（一个 struct 通吃所有 [`TableSource`]）。
pub struct DbTableProvider(pub &'static TableSource);

#[async_trait]
impl SourceProvider for DbTableProvider {
    fn kind(&self) -> &'static str {
        self.0.kind
    }
    fn trust(&self) -> &'static str {
        self.0.trust
    }

    async fn enumerate(
        &self,
        db: &Db,
        project_id: &str,
        _repo_path: Option<&str>,
        limit: i64,
        query: Option<&str>,
    ) -> Result<Vec<ContextItem>> {
        let s = self.0;
        // 组装 WHERE：项目作用域（可空=全局）+ 恒定额外谓词 + 可选标题搜索（LIKE 下推）。
        let mut conds: Vec<String> = Vec::new();
        if !s.scope_project.is_empty() {
            conds.push(format!("{} = ?", s.scope_project));
        }
        if !s.extra_where.is_empty() {
            conds.push(s.extra_where.to_string());
        }
        let query = query.map(str::trim).filter(|q| !q.is_empty());
        if query.is_some() {
            // title_sql 是受信任的静态表达式；用户 query 走 bind + ESCAPE 转义。
            conds.push(format!("({}) LIKE ? ESCAPE '\\'", s.title_sql));
        }
        let where_sql = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        // 预览表达式：默认取正文列前 200 字，JSON 正文来源可覆盖为 json_extract。
        let preview_expr = if s.preview_sql.is_empty() {
            format!("substr(t.{},1,200)", s.content_col)
        } else {
            s.preview_sql.to_string()
        };
        // 标识符类字段全部是静态字面量；运行时值走 bind。
        let sql = format!(
            "SELECT CAST(t.{id} AS TEXT) AS sid, {title} AS title, {preview} AS preview, \
                    COALESCE(length(t.{content}), 0) AS sz, t.{time} AS ts \
             FROM {from} {where_sql} ORDER BY t.{time} DESC LIMIT ?",
            id = s.id_col,
            title = s.title_sql,
            preview = preview_expr,
            content = s.content_col,
            time = s.time_col,
            from = s.scope_from,
        );
        let mut q =
            sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, Option<String>)>(&sql);
        if !s.scope_project.is_empty() {
            q = q.bind(project_id);
        }
        if let Some(qs) = query {
            q = q.bind(like_pattern(qs));
        }
        q = q.bind(limit);
        let rows = q.fetch_all(db).await?;

        Ok(rows
            .into_iter()
            .map(|(sid, title, preview, sz, ts)| {
                let ts = ts.unwrap_or_default();
                let title = clean_title(title.as_deref().unwrap_or(""));
                ContextItem {
                    id: stable_id(s.kind, &sid),
                    project_id: project_id.to_string(),
                    source_kind: s.kind.to_string(),
                    source_id: sid.clone(),
                    preview: clean_preview(preview.as_deref().unwrap_or(""), &title),
                    title,
                    origin_stage: s.stage.to_string(),
                    origin_actor: String::new(),
                    content_ref: format!("lazy:{}:{}", s.kind, sid),
                    size_hint: sz,
                    trust: s.trust.to_string(),
                    labels: "[]".to_string(),
                    created_at: ts.clone(),
                    updated_at: ts,
                }
            })
            .collect())
    }

    async fn fetch(
        &self,
        db: &Db,
        source_id: &str,
        _repo_path: Option<&str>,
        _max_chars: usize,
    ) -> Result<String> {
        let s = self.0;
        let sql = format!(
            "SELECT {content} FROM {table} WHERE {id} = ? LIMIT 1",
            content = s.content_col,
            table = s.table,
            id = s.id_col,
        );
        let raw = sqlx::query_as::<_, (Option<String>,)>(&sql)
            .bind(source_id)
            .fetch_optional(db)
            .await?
            .and_then(|(v,)| v)
            .unwrap_or_default();
        Ok(raw)
    }
}

/// `.autoforge` 文件来源的通用 provider。source_id = 相对 `.autoforge/` 的路径（如 `docs/prd.md`）。
pub struct WorkspaceFileProvider(pub &'static FileSource);

#[async_trait]
impl SourceProvider for WorkspaceFileProvider {
    fn kind(&self) -> &'static str {
        self.0.kind
    }

    async fn enumerate(
        &self,
        _db: &Db,
        project_id: &str,
        repo_path: Option<&str>,
        limit: i64,
        query: Option<&str>,
    ) -> Result<Vec<ContextItem>> {
        let Some(repo) = repo_path.filter(|r| !r.is_empty()) else {
            return Ok(vec![]); // 未配置本地仓库 → 无文件来源
        };
        let base = std::path::Path::new(repo).join(".autoforge");
        let mut rels: Vec<String> = Vec::new();
        if let Some(sub) = self.0.subdir {
            collect_files(&base.join(sub), sub, &mut rels).await;
        } else {
            for f in self.0.files {
                if base.join(f).is_file() {
                    rels.push((*f).to_string());
                }
            }
        }
        // 搜索：按相对路径子串过滤（文件来源量小，内存过滤足够）。
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            let ql = q.to_lowercase();
            rels.retain(|r| r.to_lowercase().contains(&ql));
        }
        rels.truncate(limit.max(0) as usize);

        let mut out = Vec::with_capacity(rels.len());
        for rel in rels {
            let abs = base.join(&rel);
            let meta = tokio::fs::metadata(&abs).await.ok();
            let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
            // 用文件修改时间填 created_at，格式与 DB `datetime('now')`（"%Y-%m-%d %H:%M:%S" UTC）
            // 对齐，使文件来源能与 DB 来源按时间**穿插排序**——否则空时间恒排末尾，活跃项目下
            // 会被 list 的总量截断挤出「全部来源」视图（.autoforge 文件不可见）。
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    chrono::DateTime::<chrono::Utc>::from(t)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            let title = std::path::Path::new(&rel)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&rel)
                .to_string();
            out.push(ContextItem {
                id: stable_id(self.0.kind, &rel),
                project_id: project_id.to_string(),
                source_kind: self.0.kind.to_string(),
                source_id: rel.clone(),
                // 预览用相对路径，区分不同目录下的同名文件（title 仅文件名）。
                preview: rel.clone(),
                title,
                origin_stage: self.0.stage.to_string(),
                origin_actor: String::new(),
                content_ref: format!("file:{}", abs.display()),
                size_hint: size,
                trust: trust::TRUSTED.to_string(),
                labels: "[]".to_string(),
                created_at: mtime.clone(),
                updated_at: mtime,
            });
        }
        Ok(out)
    }

    async fn fetch(
        &self,
        _db: &Db,
        source_id: &str,
        repo_path: Option<&str>,
        _max_chars: usize,
    ) -> Result<String> {
        let Some(repo) = repo_path.filter(|r| !r.is_empty()) else {
            return Ok(String::new());
        };
        // 复用 `.autoforge` 越界守卫：source_id 必须落在 .autoforge 内，禁止 `..` 逃逸。
        let base = std::path::Path::new(repo).join(".autoforge");
        let target = base.join(source_id);
        let canon_base = tokio::fs::canonicalize(&base).await.unwrap_or(base.clone());
        match tokio::fs::canonicalize(&target).await {
            Ok(canon) if canon.starts_with(&canon_base) => {
                Ok(tokio::fs::read_to_string(&canon).await.unwrap_or_default())
            }
            _ => Ok(String::new()), // 越界 / 不存在 → 空
        }
    }
}

/// 递归收集目录下的相对文件路径（相对 `.autoforge/`，即含子目录前缀）。
async fn collect_files(dir: &std::path::Path, rel_prefix: &str, out: &mut Vec<String>) {
    let mut stack = vec![(dir.to_path_buf(), rel_prefix.to_string())];
    while let Some((d, prefix)) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&d).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        while let Ok(Some(ent)) = rd.next_entry().await {
            let name = ent.file_name().to_string_lossy().to_string();
            let rel = format!("{prefix}/{name}");
            match ent.file_type().await {
                Ok(ft) if ft.is_dir() => stack.push((ent.path(), rel)),
                Ok(ft) if ft.is_file() => out.push(rel),
                _ => {}
            }
        }
    }
}

/// 标题清洗：折叠空白 / 去控制字符 / 截到 ~80 字符（枚举结果供 picker 展示）。
fn clean_title(raw: &str) -> String {
    let collapsed: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() > 80 {
        format!("{}…", trimmed.chars().take(79).collect::<String>())
    } else if trimmed.is_empty() {
        "（无标题）".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 预览片段清洗：折叠空白/控制符、截断到 160 字。空或与标题重复时回落空串（前端不渲染）。
fn clean_preview(raw: &str, title: &str) -> String {
    let collapsed: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() || trimmed == title || title.starts_with(trimmed) {
        return String::new();
    }
    if trimmed.chars().count() > 160 {
        format!("{}…", trimmed.chars().take(159).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

// ── 注册表 + 统一活查/懒取出口 ──────────────────────────────────────────────

/// 构建全部 provider（DB 表来源 + 文件来源）。轻量（只 box 静态引用），每次调用便宜。
pub fn providers() -> Vec<Box<dyn SourceProvider>> {
    let mut v: Vec<Box<dyn SourceProvider>> = Vec::with_capacity(SOURCES.len() + FILE_SOURCES.len());
    for s in SOURCES {
        v.push(Box::new(DbTableProvider(s)));
    }
    for f in FILE_SOURCES {
        v.push(Box::new(WorkspaceFileProvider(f)));
    }
    v
}

/// 活查：遍历命中的 provider（`kinds` 空 = 全部来源），合并枚举结果。
/// 各 provider 独立失败不影响整体（best-effort，坏一个来源不拖垮全量）。
pub async fn enumerate_all(
    db: &Db,
    project_id: &str,
    kinds: &[&str],
    repo_path: Option<&str>,
    per_source_limit: i64,
    query: Option<&str>,
) -> Result<Vec<ContextItem>> {
    let mut out: Vec<ContextItem> = Vec::new();
    for p in providers() {
        if !kinds.is_empty() && !kinds.contains(&p.kind()) {
            continue;
        }
        match p.enumerate(db, project_id, repo_path, per_source_limit, query).await {
            Ok(mut items) => out.append(&mut items),
            Err(e) => {
                tracing::warn!("[context] provider {} enumerate 失败: {}", p.kind(), e);
            }
        }
    }
    // 全局按时间倒序（跨来源合并后统一定序）。
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// 归属校验：`<kind>:<source_id>` 是否属于 `project_id`。
/// 供消费侧（如 read_context 工具）在 [`crate::core::context::get`] 反构出**无归属**的
/// 最小条目后补验——DB 来源的 `fetch` 本身不带项目过滤，缺这道就是跨项目读洞。
/// - DB 表来源：用声明的 `scope_from`/`scope_project` 拼 `SELECT 1` 活查；
///   `scope_project` 为空 = 全局来源（cfg_* 等），视为归属任意项目。
/// - 文件来源：`fetch` 走**本项目** repo_path 解析，外项目路径天然取不到 → 放行。
/// - 未知 kind：拒绝。
pub async fn belongs_to_project(
    db: &Db,
    kind: &str,
    source_id: &str,
    project_id: &str,
) -> Result<bool> {
    for s in SOURCES {
        if s.kind == kind {
            if s.scope_project.is_empty() {
                return Ok(true);
            }
            let sql = format!(
                "SELECT 1 FROM {from} WHERE t.{id} = ? AND {scope} = ? LIMIT 1",
                from = s.scope_from,
                id = s.id_col,
                scope = s.scope_project,
            );
            let hit = sqlx::query_as::<_, (i64,)>(&sql)
                .bind(source_id)
                .bind(project_id)
                .fetch_optional(db)
                .await?;
            return Ok(hit.is_some());
        }
    }
    for f in FILE_SOURCES {
        if f.kind == kind {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 按 kind 懒取一条正文（替代原硬编码 scheme match）。外部来源过注入闸（I4）。
pub async fn fetch_kind(
    db: &Db,
    kind: &str,
    source_id: &str,
    repo_path: Option<&str>,
    max_chars: usize,
) -> Result<Option<String>> {
    for p in providers() {
        if p.kind() == kind {
            let raw = p.fetch(db, source_id, repo_path, max_chars).await?;
            // 外部不可信来源：回灌前过注入闸；命中即抹成安全提示。
            if p.trust() == trust::EXTERNAL_UNTRUSTED
                && crate::core::security::has_obvious_injection(&raw)
            {
                return Ok(Some("（外部来源疑似注入，已拦截）".to_string()));
            }
            return Ok(Some(raw));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I3 护栏：任何 [`TableSource`] 都不得触碰黑名单表。
    #[test]
    fn sources_never_touch_blacklist() {
        for s in SOURCES {
            assert!(
                !NEVER_CONTEXT.contains(&s.table),
                "来源 {} 触碰了黑名单表 {}",
                s.kind,
                s.table
            );
            // scope_from 里 JOIN 的表也不应引到密钥表（粗查表名字面量）。
            for bl in NEVER_CONTEXT {
                assert!(
                    !s.scope_from.contains(bl),
                    "来源 {} 的 scope_from 引用了黑名单表 {}",
                    s.kind,
                    bl
                );
            }
        }
    }

    /// mcp_servers 来源绝不点名密文列（env_json/headers_json）。
    #[test]
    fn mcp_source_never_exposes_secret_columns() {
        let mcp = SOURCES.iter().find(|s| s.kind == sk::CFG_MCP).unwrap();
        assert_ne!(mcp.content_col, "env_json");
        assert_ne!(mcp.content_col, "headers_json");
        assert!(!mcp.title_sql.contains("env_json"));
        assert!(!mcp.title_sql.contains("headers_json"));
    }

    /// 标识符类字段（table/id_col/content_col/time_col）全部安全（防 typo 逃逸成注入）。
    #[test]
    fn source_identifiers_are_safe() {
        for s in SOURCES {
            assert!(is_safe_ident(s.table), "table 非法: {}", s.table);
            assert!(is_safe_ident(s.id_col), "id_col 非法: {}", s.id_col);
            assert!(is_safe_ident(s.content_col), "content_col 非法: {}", s.content_col);
            assert!(is_safe_ident(s.time_col), "time_col 非法: {}", s.time_col);
            // scope_project 可空或形如 x.y。
            if !s.scope_project.is_empty() {
                assert!(is_safe_ident(s.scope_project), "scope_project 非法: {}", s.scope_project);
            }
        }
    }

    /// LIKE 转义：通配符与转义符本身都被转义，中文原样保留。
    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("abc"), "%abc%");
        assert_eq!(like_pattern("100%"), "%100\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("a\\b"), "%a\\\\b%");
        assert_eq!(like_pattern("登录页"), "%登录页%");
    }

    #[test]
    fn is_safe_ident_rejects_injection() {
        assert!(is_safe_ident("issues"));
        assert!(is_safe_ident("t.project_id"));
        assert!(!is_safe_ident("issues; drop table x"));
        assert!(!is_safe_ident("a b"));
        assert!(!is_safe_ident(""));
        assert!(!is_safe_ident("Issues")); // 大写不允许
    }

    #[test]
    fn clean_title_collapses_and_truncates() {
        assert_eq!(clean_title("  hi\n\tthere  "), "hi there");
        assert_eq!(clean_title(""), "（无标题）");
        let long = "a".repeat(200);
        let out = clean_title(&long);
        assert!(out.chars().count() <= 80);
        assert!(out.ends_with('…'));
    }

    async fn pool() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE issues (id TEXT PRIMARY KEY, project_id TEXT, title TEXT, description TEXT, created_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE conversations (id TEXT PRIMARY KEY, project_id TEXT, deleted_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id TEXT PRIMARY KEY, conversation_id TEXT, content_json TEXT, created_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        p
    }

    /// DbTableProvider：enumerate 按项目过滤 + fetch 取正文。
    #[tokio::test]
    async fn db_provider_enumerate_and_fetch() {
        let db = pool().await;
        sqlx::query("INSERT INTO issues VALUES ('i1','p1','登录页','需要登录页面','2026-01-01')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO issues VALUES ('i2','p2','别的项目','x','2026-01-02')")
            .execute(&db)
            .await
            .unwrap();

        let issue_src = SOURCES.iter().find(|s| s.kind == sk::ISSUE).unwrap();
        let prov = DbTableProvider(issue_src);
        let items = prov.enumerate(&db, "p1", None, 100, None).await.unwrap();
        assert_eq!(items.len(), 1, "只见本项目需求");
        assert_eq!(items[0].id, "issue:i1");
        assert_eq!(items[0].title, "登录页");
        let body = prov.fetch(&db, "i1", None, 1000).await.unwrap();
        assert_eq!(body, "需要登录页面");
    }

    /// chat_message：经 conversations 一跳拿 project_id，排除软删会话。
    #[tokio::test]
    async fn chat_provider_joins_conversation_and_excludes_deleted() {
        let db = pool().await;
        sqlx::query("INSERT INTO conversations VALUES ('c1','p1',NULL)")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversations VALUES ('c2','p1','2026-01-01')") // 软删
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages VALUES ('m1','c1','[{\"t\":\"md\",\"text\":\"你好\"}]','2026-01-01')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages VALUES ('m2','c2','[{\"t\":\"md\",\"text\":\"删了\"}]','2026-01-02')")
            .execute(&db)
            .await
            .unwrap();

        let src = SOURCES.iter().find(|s| s.kind == sk::CHAT_MESSAGE).unwrap();
        let prov = DbTableProvider(src);
        let items = prov.enumerate(&db, "p1", None, 100, None).await.unwrap();
        assert_eq!(items.len(), 1, "软删会话的消息不出现");
        assert_eq!(items[0].source_id, "m1");
        // 标题从 content_json 首块提取可读文本，而非原始 JSON。
        assert_eq!(items[0].title, "你好");
    }

    /// 预览片段：DB 来源取正文前若干字，且与标题重复时回落空串。
    #[tokio::test]
    async fn db_provider_fills_preview_snippet() {
        let db = pool().await;
        sqlx::query("INSERT INTO issues VALUES ('i1','p1','登录页','需要一个带记住我的登录页面','2026-01-01')")
            .execute(&db)
            .await
            .unwrap();
        let src = SOURCES.iter().find(|s| s.kind == sk::ISSUE).unwrap();
        let items = DbTableProvider(src).enumerate(&db, "p1", None, 100, None).await.unwrap();
        assert_eq!(items[0].title, "登录页");
        assert_eq!(items[0].preview, "需要一个带记住我的登录页面", "预览取 description 正文");
    }
}
