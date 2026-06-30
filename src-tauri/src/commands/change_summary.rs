//! AI 变更摘要：基于 CR 的代码 diff 生成结构化摘要（修改文件清单 + 业务意图分类 +
//! 敏感模块高亮标签），供功能审核页的 CR 预览卡片渲染。
//!
//! 设计取舍：
//! - **文件清单** 由 Rust 直接解析 `diff --git` 头确定性产出（增/删/改/重命名），
//!   不依赖 LLM —— 保证 AC-2 即使模型失败也准确。
//! - **业务意图分类** 与 **per-file 说明** 由 LLM 语义生成（复用 `run_agent_text` 现有链路）。
//! - **敏感标签** 双保险：先关键词匹配兜底（数据库/API/权限/密钥/迁移），再并入 LLM 识别的
//!   额外敏感点，按 kind 去重 —— 即使 LLM 失败也能给出关键词级提示（见风险缓解）。
//! - 大 diff 预截断到上限再喂模型，避免超 token；截断仅影响 LLM 语义部分，文件清单仍取全量。
//!
//! 摘要为**实时生成、不落库**（需求明确「实时生成」），故无迁移、无新表。
//!
//! 安全：diff 是项目自身代码（半可信），喂给 LLM 是本功能本意，不施加 `has_obvious_injection`
//! （否则正常代码里的字面量会误伤）；LLM 产出仅作为纯文本渲染（React 默认转义），不回灌二次 prompt。

use crate::db::Db;
use crate::models::agent::Agent;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tauri::State;

/// 喂给 LLM 的 diff 上限（字符）。超出做头部截断 + 提示，避免超 token 导致整体失败。
const MAX_DIFF_CHARS: usize = 48_000;

/// 结构化变更摘要（前端 ChangeSummaryCard 消费）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChangeSummary {
    /// 概述：一句话本次变更做了什么（LLM 生成，失败时为空）。
    pub overview: String,
    /// 修改文件清单（Rust 确定性解析，始终可靠）。
    pub files: Vec<SummaryFile>,
    /// 按改动类型分类的业务意图（LLM 生成）。
    pub intents: Vec<IntentGroup>,
    /// 敏感模块高亮标签（关键词兜底 ∪ LLM 识别，按 kind 去重）。
    pub sensitive: Vec<SensitiveTag>,
    /// 生成状态：`ok` 全量成功 / `degraded` LLM 失败仅余确定性部分 / `empty` 无 diff。
    pub status: String,
    /// 降级 / 空态提示文案（前端展示），正常为 None。
    pub note: Option<String>,
}

/// 单个变更文件。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SummaryFile {
    pub path: String,
    /// `added` | `modified` | `deleted` | `renamed`
    pub change: String,
    /// LLM 给出的该文件改动简述（可空）。
    pub note: String,
}

/// 一组业务意图（按类型聚合）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct IntentGroup {
    /// 意图类型，如「功能新增」「Bug 修复」「重构」「测试」「文档」「配置」等。
    pub kind: String,
    /// 该类意图的简明说明。
    pub detail: String,
}

/// 敏感模块标签。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SensitiveTag {
    /// 机器可读类别：`database` | `api` | `permission` | `secret` | `migration`。
    pub kind: String,
    /// 展示文案，如「数据库变更」。
    pub label: String,
    /// 触发原因 / 命中位置简述。
    pub detail: String,
}

/// LLM 期望返回的 JSON 形状（仅语义部分；文件清单与兜底标签由 Rust 负责）。
#[derive(Debug, Default, Deserialize)]
struct LlmSummary {
    #[serde(default)]
    overview: String,
    #[serde(default)]
    intents: Vec<IntentGroup>,
    #[serde(default)]
    file_notes: Vec<LlmFileNote>,
    #[serde(default)]
    sensitive: Vec<LlmSensitive>,
}

#[derive(Debug, Default, Deserialize)]
struct LlmFileNote {
    #[serde(default)]
    path: String,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Default, Deserialize)]
struct LlmSensitive {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    detail: String,
}

/// IPC 命令：为指定 CR 生成 AI 变更摘要（薄包装，逻辑见 [`ensure_change_summary`]）。
/// 后端自取 diff（单一真源、避免大 diff 在 IPC 往返两次）。
#[tauri::command]
pub async fn generate_change_summary(
    cr_id: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ChangeSummary, String> {
    ensure_change_summary(&state.db, &cr_id, force.unwrap_or(false)).await
}

/// 生成（或命中缓存返回）CR 的变更摘要，成功结果落库。供 IPC 命令与执行完成后的
/// 后台预生成共用——执行结束即预热缓存，用户打开审核页直接命中、无需等待 LLM。
///
/// 缓存策略：摘要只随 diff 内容变化，按 `cr_id` 缓存并记录所依据 diff 的 sha256；命中且哈希
/// 一致则直接返回——切换需求/CR 不重跑 LLM。`force=true`（卡片「重新生成」按钮）跳过缓存强制
/// 重算。只缓存成功结果（status=ok），degraded/empty 不落库以便下次自动重试。
pub async fn ensure_change_summary(
    db: &Db,
    cr_id: &str,
    force: bool,
) -> Result<ChangeSummary, String> {
    let diff = crate::commands::change_requests::load_cr_diff(db, cr_id).await?;
    let diff_hash = hex::encode(sha2::Sha256::digest(diff.as_bytes()));

    if !force {
        if let Some(cached) = load_cached_summary(db, cr_id, &diff_hash).await {
            return Ok(cached);
        }
    }

    let summary = build_change_summary(db, &diff).await;
    // 仅缓存完整成功的摘要；降级 / 空态不落库，下次自动重试。
    if summary.status == "ok" {
        save_cached_summary(db, cr_id, &diff_hash, &summary).await;
    }
    Ok(summary)
}

/// 读取命中缓存：要求 cr_id 存在且 diff_hash 与当前 diff 一致（diff 变化则视为未命中，触发重算）。
async fn load_cached_summary(db: &Db, cr_id: &str, diff_hash: &str) -> Option<ChangeSummary> {
    let json: String = sqlx::query_scalar(
        "SELECT summary_json FROM change_summaries WHERE cr_id = ? AND diff_hash = ?",
    )
    .bind(cr_id)
    .bind(diff_hash)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;
    serde_json::from_str(&json).ok()
}

/// 写入 / 覆盖缓存（按 cr_id upsert，diff 变化时新哈希覆盖旧摘要）。失败仅记日志，不影响返回。
async fn save_cached_summary(db: &Db, cr_id: &str, diff_hash: &str, summary: &ChangeSummary) {
    let json = match serde_json::to_string(summary) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[change_summary] 摘要序列化失败，跳过缓存: {e}");
            return;
        }
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO change_summaries (cr_id, diff_hash, summary_json, created_at)
         VALUES (?, ?, ?, datetime('now'))
         ON CONFLICT(cr_id) DO UPDATE SET
           diff_hash = excluded.diff_hash,
           summary_json = excluded.summary_json,
           created_at = excluded.created_at",
    )
    .bind(cr_id)
    .bind(diff_hash)
    .bind(&json)
    .execute(db)
    .await
    {
        eprintln!("[change_summary] 缓存写入失败（不影响展示）: {e}");
    }
}

/// 组装摘要（纯逻辑，便于单测）：确定性文件清单 + 关键词敏感标签 + 可选 LLM 语义增强。
async fn build_change_summary(db: &Db, diff: &str) -> ChangeSummary {
    let files_raw = parse_diff_files(diff);
    if files_raw.is_empty() {
        return ChangeSummary {
            status: "empty".into(),
            note: Some("Diff 为空或 worktree 不存在，暂无可摘要的变更。".into()),
            ..Default::default()
        };
    }

    // 确定性兜底：关键词敏感标签 + 文件清单（无 note）。
    let mut sensitive = keyword_sensitive_tags(diff);
    let mut files: Vec<SummaryFile> = files_raw
        .iter()
        .map(|(path, change)| SummaryFile {
            path: path.clone(),
            change: change.clone(),
            note: String::new(),
        })
        .collect();

    // 语义增强：调 LLM。失败则降级——仍返回确定性部分，绝不让卡片崩。
    let (truncated_diff, was_truncated) = truncate_diff(diff);
    match run_llm_summary(db, &files_raw, &truncated_diff, was_truncated).await {
        Ok(llm) => {
            // 把 LLM 的 per-file 说明回填到文件清单（按 path 匹配）。
            for note in llm.file_notes {
                if note.note.trim().is_empty() {
                    continue;
                }
                if let Some(f) = files.iter_mut().find(|f| f.path == note.path) {
                    f.note = note.note.trim().to_string();
                }
            }
            // 并入 LLM 识别的敏感点（按 kind 去重，关键词兜底优先保留）。
            for s in llm.sensitive {
                let kind = s.kind.trim().to_lowercase();
                if kind.is_empty() || sensitive.iter().any(|t| t.kind == kind) {
                    continue;
                }
                let label = if s.label.trim().is_empty() {
                    default_sensitive_label(&kind)
                } else {
                    s.label.trim().to_string()
                };
                sensitive.push(SensitiveTag {
                    kind,
                    label,
                    detail: s.detail.trim().to_string(),
                });
            }
            let intents: Vec<IntentGroup> = llm
                .intents
                .into_iter()
                .filter(|g| !g.kind.trim().is_empty() || !g.detail.trim().is_empty())
                .collect();
            ChangeSummary {
                overview: llm.overview.trim().to_string(),
                files,
                intents,
                sensitive,
                status: "ok".into(),
                note: None,
            }
        }
        Err(e) => {
            eprintln!("[change_summary] LLM 摘要生成失败，降级为确定性摘要: {e}");
            ChangeSummary {
                overview: String::new(),
                files,
                intents: Vec::new(),
                sensitive,
                status: "degraded".into(),
                note: Some("AI 语义摘要暂不可用，仅展示文件清单与关键词敏感标签。".into()),
            }
        }
    }
}

/// 解析 `diff --git` 头，确定每个文件的路径与改动类型。返回 (path, change)。
fn parse_diff_files(diff: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("diff --git ") {
            continue;
        }
        // 形如：diff --git a/foo/bar.rs b/foo/bar.rs
        let a_path = parse_git_header_path(line);
        let mut change = "modified".to_string();
        let mut path = a_path.clone();
        // 读取该文件块的元信息行，判定增/删/重命名，直到下一个 diff --git。
        while let Some(peek) = lines.peek() {
            if peek.starts_with("diff --git ") {
                break;
            }
            let meta = lines.next().unwrap();
            if meta.starts_with("new file mode") {
                change = "added".into();
            } else if meta.starts_with("deleted file mode") {
                change = "deleted".into();
            } else if meta.starts_with("rename to ") {
                change = "renamed".into();
                path = meta.trim_start_matches("rename to ").trim().to_string();
            } else if meta.starts_with("+++ b/") {
                // 用 +++ 行确认最终路径（覆盖 header 解析的歧义，如带空格路径）。
                let p = meta.trim_start_matches("+++ b/").trim();
                if p != "/dev/null" && !p.is_empty() {
                    path = p.to_string();
                }
            } else if meta.starts_with("--- a/") && change == "deleted" {
                let p = meta.trim_start_matches("--- a/").trim();
                if p != "/dev/null" && !p.is_empty() {
                    path = p.to_string();
                }
            }
        }
        if !path.is_empty() {
            out.push((path, change));
        }
    }
    out
}

/// 从 `diff --git a/X b/X` 行提取路径（取 b/ 侧，回退 a/ 侧）。
fn parse_git_header_path(line: &str) -> String {
    let rest = line.trim_start_matches("diff --git ").trim();
    // 优先取 " b/" 之后的部分。
    if let Some(idx) = rest.find(" b/") {
        return rest[idx + 3..].trim().to_string();
    }
    if let Some(stripped) = rest.strip_prefix("a/") {
        return stripped.trim().to_string();
    }
    rest.to_string()
}

/// 关键词敏感标签兜底：扫描 diff 命中数据库/API/权限/密钥/迁移类改动。
/// 仅在「新增/删除行（+/-）」上判定，避免上下文行误报。
fn keyword_sensitive_tags(diff: &str) -> Vec<SensitiveTag> {
    let mut hits: Vec<SensitiveTag> = Vec::new();

    // 仅看实际改动行（+ / -，排除 +++ / --- 文件头）。
    let changed: String = diff
        .lines()
        .filter(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lower = changed.to_lowercase();
    let file_lines: Vec<String> = diff
        .lines()
        .filter(|l| l.starts_with("diff --git "))
        .map(|l| parse_git_header_path(l).to_lowercase())
        .collect();
    let any_path = |needle: &str| file_lines.iter().any(|p| p.contains(needle));

    // 数据库（用强 SQL 信号，避免 "update state" 之类误报；语义层由 LLM 兜底补全）
    if any_path("migrations/")
        || lower.contains("create table")
        || lower.contains("alter table")
        || lower.contains("sqlx::query")
        || lower.contains("insert into")
        || lower.contains("delete from")
        || lower.contains("select * from")
        || lower.contains("update set")
    {
        push_tag(&mut hits, "database", "数据库变更", "命中 SQL / 迁移相关改动");
    }
    // 迁移（独立高亮：迁移文件不可逆，单列提醒）
    if any_path("/migrations/") || any_path("migrations/") {
        push_tag(
            &mut hits,
            "migration",
            "数据库迁移",
            "改动涉及 migrations/ 目录（迁移仅增不改）",
        );
    }
    // API / IPC 接口
    if lower.contains("#[tauri::command]")
        || lower.contains("invoke_handler")
        || lower.contains("ipc<")
        || lower.contains("invoke<")
        || any_path("capabilities/")
        || any_path("services/index.ts")
    {
        push_tag(&mut hits, "api", "API / IPC 变更", "命中 Tauri command / IPC 封装改动");
    }
    // 权限 / 鉴权（避开 "auth"→"author"、裸 "role"/"token" 之类噪声，用更具体的信号）
    if lower.contains("permission")
        || lower.contains("capabilit")
        || lower.contains("authorize")
        || lower.contains("authenticate")
        || lower.contains("rbac")
        || lower.contains("allowtools")
        || lower.contains("allowed_tools")
        || lower.contains("disallowedtools")
    {
        push_tag(&mut hits, "permission", "权限控制变更", "命中权限 / 鉴权 / 能力声明相关改动");
    }
    // 密钥 / 加密
    if lower.contains("api_key")
        || lower.contains("secret")
        || lower.contains("encrypt")
        || lower.contains("decrypt")
        || lower.contains("password")
        || lower.contains("keyring")
    {
        push_tag(&mut hits, "secret", "密钥 / 加密变更", "命中密钥 / 加解密相关改动");
    }
    hits
}

fn push_tag(hits: &mut Vec<SensitiveTag>, kind: &str, label: &str, detail: &str) {
    if hits.iter().any(|t| t.kind == kind) {
        return;
    }
    hits.push(SensitiveTag {
        kind: kind.to_string(),
        label: label.to_string(),
        detail: detail.to_string(),
    });
}

fn default_sensitive_label(kind: &str) -> String {
    match kind {
        "database" => "数据库变更",
        "migration" => "数据库迁移",
        "api" => "API / IPC 变更",
        "permission" => "权限控制变更",
        "secret" => "密钥 / 加密变更",
        _ => "敏感变更",
    }
    .to_string()
}

/// 截断 diff 到上限（头部保留）。返回 (截断后文本, 是否发生截断)。
fn truncate_diff(diff: &str) -> (String, bool) {
    if diff.chars().count() <= MAX_DIFF_CHARS {
        return (diff.to_string(), false);
    }
    let head: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    (head, true)
}

/// 调 LLM 生成语义摘要。解析失败 / 无可用 Agent 均返回 Err，交由上层降级。
async fn run_llm_summary(
    db: &Db,
    files: &[(String, String)],
    diff: &str,
    truncated: bool,
) -> anyhow::Result<LlmSummary> {
    let agent = resolve_summary_agent(db)
        .await
        .ok_or_else(|| anyhow::anyhow!("未找到可用于摘要的 Agent（需有启用且绑定 LLM 的 Agent）"))?;

    let file_list = files
        .iter()
        .map(|(p, c)| format!("- [{c}] {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    let trunc_note = if truncated {
        "\n\n注意：diff 过大已截断，仅给出前一部分，请基于可见内容尽力概括。"
    } else {
        ""
    };
    let prompt = format!(
        "下面是一次代码变更的 git diff 与文件清单，请生成结构化的变更摘要，**只输出 JSON**，不要解释、不要代码块围栏。\n\n\
         JSON 形状：\n\
         {{\n  \
           \"overview\": \"一句话概述本次变更整体做了什么\",\n  \
           \"intents\": [{{\"kind\": \"业务意图类型(如 功能新增/Bug修复/重构/测试/文档/配置/性能/安全)\", \"detail\": \"该类意图的简明说明\"}}],\n  \
           \"file_notes\": [{{\"path\": \"与文件清单完全一致的路径\", \"note\": \"该文件改动的一句话说明\"}}],\n  \
           \"sensitive\": [{{\"kind\": \"database|api|permission|secret|migration\", \"label\": \"展示文案\", \"detail\": \"触发原因/位置\"}}]\n\
         }}\n\n\
         要求：intents 按改动类型聚合分组；sensitive 只在确有数据库操作/接口变更/权限控制/密钥处理时给出，没有则空数组；file_notes 的 path 必须来自下方清单。{trunc_note}\n\n\
         === 文件清单 ===\n{file_list}\n\n=== diff ===\n{diff}"
    );

    let sys = "你是资深代码审查助手，擅长从 diff 中提炼业务意图并识别敏感模块改动。务必严格输出符合给定形状的 JSON。";
    let raw = crate::agents::llm::run_agent_text(db, &agent, &prompt, Some(sys), &[]).await?;

    let json = crate::agents::schema::extract_json(&raw)
        .ok_or_else(|| anyhow::anyhow!("LLM 输出中未找到 JSON 对象"))?;
    let parsed: LlmSummary = serde_json::from_str(json)?;
    Ok(parsed)
}

/// 选一个用于摘要的 Agent：优先持 `analysis` forge_role 且绑定 LLM 的（语义最贴近代码分析），
/// 否则取首个启用且绑定 LLM 的 Agent。两者皆无则 None（上层降级为确定性摘要）。
async fn resolve_summary_agent(db: &Db) -> Option<Agent> {
    if let Some(a) = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE (',' || COALESCE(forge_role, '') || ',') LIKE '%,analysis,%'
           AND enabled=1 AND llm_id IS NOT NULL
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    {
        return Some(a);
    }
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE enabled=1 AND llm_id IS NOT NULL
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/foo.rs b/src/foo.rs\n\
index 111..222 100644\n\
--- a/src/foo.rs\n\
+++ b/src/foo.rs\n\
@@ -1,3 +1,4 @@\n\
 fn a() {}\n\
+fn b() {}\n\
diff --git a/src-tauri/migrations/0099_x.sql b/src-tauri/migrations/0099_x.sql\n\
new file mode 100644\n\
index 000..333\n\
--- /dev/null\n\
+++ b/src-tauri/migrations/0099_x.sql\n\
@@ -0,0 +1,2 @@\n\
+CREATE TABLE foo (id TEXT);\n\
diff --git a/old.txt b/old.txt\n\
deleted file mode 100644\n\
index 444..000\n\
--- a/old.txt\n\
+++ /dev/null\n\
@@ -1 +0,0 @@\n\
-gone\n";

    #[test]
    fn parses_file_list_with_change_types() {
        let files = parse_diff_files(SAMPLE);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], ("src/foo.rs".into(), "modified".into()));
        assert_eq!(
            files[1],
            ("src-tauri/migrations/0099_x.sql".into(), "added".into())
        );
        assert_eq!(files[2], ("old.txt".into(), "deleted".into()));
    }

    #[test]
    fn keyword_tags_detect_db_and_migration() {
        let tags = keyword_sensitive_tags(SAMPLE);
        assert!(tags.iter().any(|t| t.kind == "database"));
        assert!(tags.iter().any(|t| t.kind == "migration"));
    }

    #[test]
    fn empty_diff_has_no_files() {
        assert!(parse_diff_files("").is_empty());
    }

    #[test]
    fn truncate_respects_limit() {
        let small = "abc";
        let (t, was) = truncate_diff(small);
        assert_eq!(t, "abc");
        assert!(!was);
    }

    #[test]
    fn permission_keyword_detected() {
        let d = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n+let token = authorize();\n";
        let tags = keyword_sensitive_tags(d);
        assert!(tags.iter().any(|t| t.kind == "permission"));
    }
}
