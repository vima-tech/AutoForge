//! L4 角色取景框（Lens）——方法论平台/基质设计 §4.3、§6。
//!
//! 取景框 = 一组「默认纳入的 source_kind + labels 过滤 + 体积预算」。同一上下文基质配不同
//! Lens 即得不同角色台子（产品/设计/编码/测试/运维/会议室），**不为每个角色重造 App**。
//! 角色收敛体现在**默认值**上而非硬隔离：默认只给该看的，基质随时可展开取用更多。
//!
//! 本模块只做「预设数据模型 + 角色默认预设 + Lens→装配请求」的纯 Rust 桥；持久化的个人预设
//! （挂 app_settings，见 [`load_presets`]）与前端台子切换 UI 另做。不引用任何 Tauri 类型。

use crate::core::context::{self, source_kind as sk, ContextRequest};
use crate::db::Db;
use crate::models::context_item::ContextItem;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 一个取景框预设。角色默认预设为系统种子；个人预设为用户派生（同结构，owner 区分）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensPreset {
    /// 稳定 id（角色 key，或个人预设的派生 id）。
    pub id: String,
    /// 人可读台子名。
    pub name: String,
    /// 角色 key：product / design / coding / testing / ops / meeting。
    pub role: String,
    /// 默认纳入的 source_kind 集合（空 = 不限来源，会议室取全部）。
    pub include: Vec<String>,
    /// 默认体积预算（字节，0 = 不限）。
    pub budget_bytes: i64,
}

impl LensPreset {
    /// Lens → 一次上下文装配请求（可再叠加显式 refs）。
    pub fn to_request(&self, project_id: &str, refs: Vec<String>) -> ContextRequest {
        ContextRequest {
            project_id: project_id.to_string(),
            include: self.include.clone(),
            refs,
            budget_bytes: self.budget_bytes,
        }
    }
}

/// 系统种子：6 个角色台子的默认取景框（§4.3「默认取景框」列）。
/// 默认预算 32KB（会议室不限）；会议室 include 为空 = 全部来源的跨角色协作场。
pub fn default_presets() -> Vec<LensPreset> {
    let p = |id: &str, name: &str, role: &str, include: &[&str], budget: i64| LensPreset {
        id: id.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        include: include.iter().map(|s| s.to_string()).collect(),
        budget_bytes: budget,
    };
    vec![
        p(
            "product",
            "产品台",
            "product",
            &[sk::ISSUE, sk::CHAT_MESSAGE, sk::INCUBATOR_DRAFT, sk::WORKSPACE_DOC],
            32_000,
        ),
        p(
            "design",
            "设计台",
            "design",
            &[sk::ISSUE, sk::MATERIAL, sk::WORKSPACE_DOC],
            32_000,
        ),
        p(
            "coding",
            "编码台",
            "coding",
            &[sk::ISSUE, sk::WORKSPACE_SPEC, sk::CODE_AGENT_LOG, sk::LLM_TRACE],
            32_000,
        ),
        p(
            "testing",
            "测试台",
            "testing",
            &[sk::CR_REVIEW, sk::CODE_AGENT_LOG, sk::WORKSPACE_SPEC],
            32_000,
        ),
        p(
            "ops",
            "实施运维台",
            "ops",
            &[sk::CR_REVIEW, sk::WORKSPACE_DELIVERABLE],
            32_000,
        ),
        // 会议室：跨角色协作场，include 为空 = 全部来源；预算放宽。
        p("meeting", "会议室", "meeting", &[], 0),
    ]
}

/// 取某角色的默认取景框（未知角色回落「会议室=全部」，永不失败）。
pub fn preset_for_role(role: &str) -> LensPreset {
    default_presets()
        .into_iter()
        .find(|p| p.role == role)
        .unwrap_or_else(|| LensPreset {
            id: "all".into(),
            name: "全部".into(),
            role: "meeting".into(),
            include: vec![],
            budget_bytes: 0,
        })
}

/// 有效取景框列表：系统种子 ∪ 个人预设（app_settings 键 `lens_presets` 存 JSON 数组）。
/// 无个人预设时只回种子。个人预设按 id 覆盖同名种子。
pub async fn load_presets(db: &Db) -> Vec<LensPreset> {
    let mut presets = default_presets();
    let personal: Option<String> =
        sqlx::query_as::<_, (String,)>("SELECT value FROM app_settings WHERE key='lens_presets'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .map(|(v,)| v);
    if let Some(json) = personal {
        if let Ok(extra) = serde_json::from_str::<Vec<LensPreset>>(&json) {
            for e in extra {
                if let Some(slot) = presets.iter_mut().find(|p| p.id == e.id) {
                    *slot = e; // 个人覆盖同 id 种子
                } else {
                    presets.push(e);
                }
            }
        }
    }
    presets
}

/// 保存/更新一个个人取景框预设（挂 app_settings 键 `lens_presets` 的 JSON 数组，按 id upsert）。
/// 个人预设可覆盖同 id 的系统种子（[`load_presets`] 合并时生效）。
pub async fn save_preset(db: &Db, preset: LensPreset) -> Result<()> {
    let mut personal: Vec<LensPreset> =
        sqlx::query_as::<_, (String,)>("SELECT value FROM app_settings WHERE key='lens_presets'")
            .fetch_optional(db)
            .await?
            .and_then(|(v,)| serde_json::from_str(&v).ok())
            .unwrap_or_default();
    if let Some(slot) = personal.iter_mut().find(|p| p.id == preset.id) {
        *slot = preset;
    } else {
        personal.push(preset);
    }
    let json = serde_json::to_string(&personal)?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('lens_presets', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
    )
    .bind(&json)
    .execute(db)
    .await?;
    Ok(())
}

/// 按取景框装配上下文条目（Lens → 装配请求 → 基质 assemble）。
pub async fn assemble_by_lens(
    db: &Db,
    preset: &LensPreset,
    project_id: &str,
    refs: Vec<String>,
) -> Result<Vec<ContextItem>> {
    context::assemble(db, &preset.to_request(project_id, refs)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_default_role_stations() {
        let ps = default_presets();
        assert_eq!(ps.len(), 6);
        let roles: Vec<&str> = ps.iter().map(|p| p.role.as_str()).collect();
        for r in ["product", "design", "coding", "testing", "ops", "meeting"] {
            assert!(roles.contains(&r), "缺角色台子 {r}");
        }
        // 编码台默认纳入编码执行日志 + trace（近期过程信息）。
        let coding = ps.iter().find(|p| p.role == "coding").unwrap();
        assert!(coding.include.contains(&sk::CODE_AGENT_LOG.to_string()));
        // 会议室 = 全部来源（include 空）。
        let meeting = ps.iter().find(|p| p.role == "meeting").unwrap();
        assert!(meeting.include.is_empty());
    }

    #[test]
    fn preset_for_unknown_role_falls_back_to_all() {
        let p = preset_for_role("nope");
        assert!(p.include.is_empty(), "未知角色回落全部来源");
    }

    #[tokio::test]
    async fn save_preset_upserts_and_load_merges() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT)")
            .execute(&db)
            .await
            .unwrap();
        // 存一个覆盖「编码台」种子的个人预设（改 include）。
        save_preset(
            &db,
            LensPreset {
                id: "coding".into(),
                name: "我的编码台".into(),
                role: "coding".into(),
                include: vec![sk::ISSUE.to_string()],
                budget_bytes: 100,
            },
        )
        .await
        .unwrap();
        let merged = load_presets(&db).await;
        let coding = merged.iter().find(|p| p.id == "coding").unwrap();
        assert_eq!(coding.name, "我的编码台", "个人预设覆盖同 id 种子");
        assert_eq!(coding.include, vec![sk::ISSUE.to_string()]);
        // 仍有 6 个（覆盖非新增）。
        assert_eq!(merged.len(), 6);
    }

    #[tokio::test]
    async fn assemble_by_lens_filters_by_role_kinds() {
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
        // 一条编码日志（编码台该看）+ 一条物料（编码台不看）。
        context::register(
            &db,
            context::NewContextItem::trusted("p1", sk::CODE_AGENT_LOG, "l1", "日志", "clog:l1"),
        )
        .await
        .unwrap();
        context::register(
            &db,
            context::NewContextItem::trusted("p1", sk::MATERIAL, "m1", "物料", "file:a"),
        )
        .await
        .unwrap();

        let coding = preset_for_role("coding");
        let items = assemble_by_lens(&db, &coding, "p1", vec![]).await.unwrap();
        let kinds: Vec<&str> = items.iter().map(|i| i.source_kind.as_str()).collect();
        assert!(kinds.contains(&sk::CODE_AGENT_LOG), "编码台纳入日志");
        assert!(!kinds.contains(&sk::MATERIAL), "编码台不纳入物料");
    }
}
