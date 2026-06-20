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

/// 从任意 LLM 文本中切出最外层 `[...]` 数组（容忍前后解释文字 / 代码块围栏）。
/// 供 1→N 的环节 agent（triage / proposer）解析批量产出。
pub fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if start >= end {
        return None;
    }
    Some(&text[start..=end])
}

/// 把 LLM 文本解析为元素类型 `T` 的数组；坏元素逐个跳过，绝不因个别坏元素丢全部。
/// 返回 `(已解析元素, status)`：`ok`=数组完整解析；`partial`=部分元素坏；`error`=非数组/全坏（回退空 Vec）。
/// 供 triage / proposer 等批量（1→N）schema agent 使用。
pub fn parse_array_or_empty<T: DeserializeOwned>(text: &str) -> (Vec<T>, ParseStatus) {
    let Some(arr_text) = extract_json_array(text) else {
        return (vec![], "error");
    };
    let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(arr_text)
    else {
        return (vec![], "error");
    };
    let total = items.len();
    let mut out = Vec::with_capacity(total);
    for el in items {
        if let Ok(v) = serde_json::from_value::<T>(el) {
            out.push(v);
        }
    }
    let status = if total == 0 {
        "ok" // 模型明确返回空数组（如「无可整理项」）是合法结论，非错误。
    } else if out.is_empty() {
        "error"
    } else if out.len() < total {
        "partial"
    } else {
        "ok"
    };
    (out, status)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, Deserialize, PartialEq)]
    struct Sample {
        #[serde(default)]
        a: String,
        #[serde(default)]
        n: i64,
    }

    #[test]
    fn extract_json_tolerates_surrounding_text() {
        let t = "解释\n```json\n{\"a\":\"x\",\"n\":3}\n```\n尾部";
        assert_eq!(extract_json(t), Some("{\"a\":\"x\",\"n\":3}"));
    }

    #[test]
    fn parse_or_default_falls_back_on_garbage() {
        let (v, st) = parse_or_default::<Sample>("not json at all");
        assert_eq!(st, "error");
        assert_eq!(v, Sample::default());
    }

    #[test]
    fn parse_array_ok_partial_and_error() {
        // 全部合法 → ok
        let (v, st) = parse_array_or_empty::<Sample>("前缀 [{\"a\":\"x\"},{\"n\":2}] 后缀");
        assert_eq!(st, "ok");
        assert_eq!(v.len(), 2);

        // 含一个坏元素（数组里塞标量）→ partial，坏元素被跳过
        let (v, st) = parse_array_or_empty::<Sample>("[{\"a\":\"ok\"}, 42]");
        assert_eq!(st, "partial");
        assert_eq!(v.len(), 1);

        // 非数组 → error 且空
        let (v, st) = parse_array_or_empty::<Sample>("{\"a\":\"x\"}");
        assert_eq!(st, "error");
        assert!(v.is_empty());

        // 空数组是合法结论 → ok
        let (v, st) = parse_array_or_empty::<Sample>("[]");
        assert_eq!(st, "ok");
        assert!(v.is_empty());
    }
}
