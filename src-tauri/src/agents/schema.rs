//! Schema 驱动 Agent 的可复用脚手架。**纯 Rust、零 Tauri 类型**，仅依赖 `Db`。
//!
//! 每个环节 agent 把"自由 prompt → 自由文本"升级为"版本化 schema 既约束推理、又结构化沉淀"。
//! 一套 schema 一物三用：① 执行标准（[`StructuredSchema::prompt_contract`] 把 schema 摊进 prompt，
//! 强制 agent 覆盖全部分析角度）；② 优化信息源（[`record`] 把强类型产出落到 `agent_outputs`）；
//! ③ 优化杠杆（`role + schema_version` 支撑字段级体检 / 版本 A/B / 失败回灌）。
//!
//! 样板抽取自 `analysis.rs`（首个 schema agent），使后续接入只需写 schema + struct + prompt。

use crate::db::Db;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// 单字段入库上限，避免超大产出撑爆 `agent_outputs`。
const MAX_FIELD_CHARS: usize = 32_000;

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_FIELD_CHARS {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX_FIELD_CHARS).collect();
        format!("{head}\n…[已截断 {} 字符]", s.chars().count() - MAX_FIELD_CHARS)
    }
}

/// 从任意 LLM 文本中切出最外层 `{...}` JSON 对象（容忍前后解释文字 / 代码块围栏）。
pub fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if start > end {
        return None;
    }
    Some(&text[start..=end])
}

/// 解析状态：`ok` 解析成功；`error` 完全失败回退 [`Default`]。
/// （字段级"partial"由各 agent 依领域完备度自行判定后传给 [`record`]。）
pub type ParseStatus = &'static str;

/// 把 LLM 文本解析为 schema 类型 `T`；失败回退 `T::default()` 并返回状态，不阻断主流程。
/// 所有 schema struct 的字段都应带 `#[serde(default)]`，保证模型漏字段时降质量而非报错。
pub fn parse_or_default<T: DeserializeOwned + Default>(text: &str) -> (T, ParseStatus) {
    match extract_json(text).and_then(|j| serde_json::from_str::<T>(j).ok()) {
        Some(v) => (v, "ok"),
        None => (T::default(), "error"),
    }
}

/// schema 驱动 agent 的输出契约：版本 + 模板 + 由模板派生的 prompt 块。
/// 让"执行标准（prompt）"与"落库结构（struct）"同源，杜绝两者漂移。
pub trait StructuredSchema: DeserializeOwned + Default {
    /// 环节标识（落 `agent_outputs.role`）。
    const ROLE: &'static str;
    /// schema 版本（落 `agent_outputs.schema_version`，支撑版本对比）。
    const VERSION: &'static str;
    /// schema 的 JSON 模板片段（带字段注释），内嵌进 prompt 作执行标准。
    fn schema_template() -> &'static str;

    /// 渲染为 prompt 的"输出契约"块——agent 据此产出严格符合 schema 的 JSON。
    fn prompt_contract() -> String {
        format!(
            "## 输出契约（schema {role} v{ver}，务必严格遵守）\n\
             只输出一个 JSON 对象（不要 markdown 代码块、不要任何解释文字），结构如下：\n\
             {tpl}\n\n\
             要求：\n\
             - 覆盖全部字段；信息不足的字段给空字符串/空数组/null，并在相应说明字段标注，绝不臆造。\n\
             - 数值字段遵守标注范围；枚举字段只用给定取值。",
            role = Self::ROLE,
            ver = Self::VERSION,
            tpl = Self::schema_template(),
        )
    }
}

/// 把一次环节 agent 的结构化产出落库到统一表 `agent_outputs`。best-effort：失败只记日志。
/// 返回新行 id。`trace_id` 传 [`crate::core::trace::current_trace_id`] 的结果即可链回单步推理。
#[allow(clippy::too_many_arguments)]
pub async fn record(
    db: &Db,
    role: &str,
    schema_version: &str,
    target_kind: &str,
    target_id: &str,
    project_id: Option<&str>,
    trace_id: Option<&str>,
    status: &str,
    output_json: &str,
    raw: &str,
) -> String {
    let id = Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO agent_outputs
         (id, role, schema_version, target_kind, target_id, project_id, trace_id, status, output_json, raw)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(role)
    .bind(schema_version)
    .bind(target_kind)
    .bind(target_id)
    .bind(project_id)
    .bind(trace_id)
    .bind(status)
    .bind(truncate(output_json))
    .bind(truncate(raw))
    .execute(db)
    .await;
    if let Err(e) = res {
        eprintln!("[agent_outputs] 写入失败（已忽略）：{}", e);
    }
    id
}
