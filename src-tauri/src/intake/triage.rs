//! 待整理池（triage）的整理引擎 —— **纯 Rust、零 Tauri 类型**。
//!
//! 把 triage Agent 的批量整理逻辑从 `commands/intake.rs` 下沉到这里，使得
//! 它既能被人工触发的 `refine_triage` 命令复用，也能被工厂自喂料
//! (`tasks/autosupply.rs`) 在**入池后立即前置去噪**——噪音不必等人点「整理」
//! 才被滤掉（见 [`denoise_in_place`]）。
//!
//! 解耦铁律：本模块只依赖 `Db` 与 `Issue`，不触碰 `AppHandle`/`State`/事件发射。
//! 命令层负责在整理结果上做「进流水线 + 发事件」这类带 Tauri 类型的动作。

use crate::agents::schema::{self, StructuredSchema};
use crate::db::Db;
use crate::models::issue::Issue;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_category() -> String {
    "Feature".to_string()
}
fn default_severity() -> String {
    "medium".to_string()
}

/// 一条碎片整理后的归一化结果（triage schema v1.0）。
/// 兼作 [`StructuredSchema`] 的强类型 spec：批量时为数组元素，单条时为对象。
/// 所有字段带 `#[serde(default)]`，模型漏字段时降质量而非报错。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriageParsed {
    /// 批量模式下回指输入序号（落库时换成 issue.id）；单条模式可空。
    #[serde(default)]
    pub idx: Option<usize>,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub description: String,
    /// 0~1：原始输入的可执行清晰度。
    #[serde(default)]
    pub clarity_score: Option<f64>,
    /// 红线字段：信息不足应追问澄清而非静默猜测。
    #[serde(default)]
    pub needs_clarification: bool,
    /// 为可执行所缺失的关键信息点。
    #[serde(default)]
    pub missing_info: Vec<String>,
    /// 疑似重复的需求线索（或 null）。
    #[serde(default)]
    pub duplicate_of: Option<String>,
    /// triage Agent 判定该碎片为噪音/无价值，应直接丢弃。
    #[serde(default)]
    pub is_noise: bool,
}

impl Default for TriageParsed {
    fn default() -> Self {
        Self {
            idx: None,
            title: String::new(),
            category: default_category(),
            severity: default_severity(),
            description: String::new(),
            clarity_score: None,
            needs_clarification: false,
            missing_info: vec![],
            duplicate_of: None,
            is_noise: false,
        }
    }
}

const TRIAGE_SCHEMA_TEMPLATE: &str = r#"{
  "title": "<简洁需求标题>", "category": "<Feature|Bug|Improvement|Debt>",
  "severity": "<critical|high|medium|low>", "description": "<补全后的清晰描述>",
  "clarity_score": <0.0-1.0>, "needs_clarification": <bool>,
  "missing_info": ["<缺失的关键信息>"], "duplicate_of": <线索或 null>,
  "is_noise": <bool>
}"#;

impl StructuredSchema for TriageParsed {
    const ROLE: &'static str = "triage";
    const VERSION: &'static str = "1.0";
    fn schema_template() -> &'static str {
        TRIAGE_SCHEMA_TEMPLATE
    }
}

/// 校验解析结果：trim title；title 为空且非噪音视为无效（返回 None）。
fn validate(mut p: TriageParsed) -> Option<TriageParsed> {
    p.title = p.title.trim().to_string();
    if p.title.is_empty() && !p.is_noise {
        return None;
    }
    Some(p)
}

/// 前置去噪的统计结果。
#[derive(Debug, Default, Clone, Copy)]
pub struct DenoiseStats {
    /// 判为噪音、已从 triage 池删除的条数。
    pub discarded: u32,
    /// 判为有效、已就地补全（仍留在 triage 池）的条数。
    pub normalized: u32,
    /// LLM/解析失败、原样保留待人工处理的条数。
    pub errors: u32,
}

/// 解析批量 triage 输出的 JSON 数组：返回 `idx -> TriageParsed` 映射（类型化 + 校验）。
/// 每个元素需带 `idx` 整数指向输入序号；缺失/坏元素被跳过，由上层对未命中的 idx
/// 回退到单条整理，保证不丢需求。
fn parse_triage_batch(out: &str) -> HashMap<usize, TriageParsed> {
    let (items, _status) = schema::parse_array_or_empty::<TriageParsed>(out);
    let mut map = HashMap::new();
    for item in items {
        let idx = item.idx;
        if let (Some(idx), Some(parsed)) = (idx, validate(item)) {
            map.insert(idx, parsed);
        }
    }
    map
}

/// 取一条碎片用于整理的原始文本：优先 raw_capture，否则回退 title+description。
fn triage_raw(issue: &Issue) -> String {
    issue
        .raw_capture
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}\n{}", issue.title, issue.description))
}

/// 单条整理：调 triage Agent → 类型化解析 + 校验，并把结构化产出落 `agent_outputs`。
/// 失败返回 None（计为出错）。
async fn refine_triage_one(db: &Db, issue: &Issue) -> Option<TriageParsed> {
    let (raw, trace_id) = crate::agents::llm::run_system_role_text_traced(
        db,
        "triage",
        &triage_raw(issue),
        None,
        Some(&issue.project_id),
        None,
    )
    .await
    .ok()?;

    let (parsed, mut status) = schema::parse_or_default::<TriageParsed>(&raw);
    let valid = validate(parsed.clone());
    if valid.is_none() {
        status = "error";
    }
    record_triage(db, issue, &parsed, status, trace_id.as_deref(), &raw).await;
    valid
}

/// 把一条 triage 结构化产出落 `agent_outputs`（role=triage, target=issue）。best-effort。
async fn record_triage(
    db: &Db,
    issue: &Issue,
    parsed: &TriageParsed,
    status: &str,
    trace_id: Option<&str>,
    raw: &str,
) {
    let output_json = serde_json::to_string(parsed).unwrap_or_else(|_| "{}".to_string());
    schema::record(
        db,
        TriageParsed::ROLE,
        TriageParsed::VERSION,
        "issue",
        &issue.id,
        Some(&issue.project_id),
        trace_id,
        status,
        &output_json,
        raw,
    )
    .await;
}

/// 一个小批的整理：单条直接走单请求；多条拼成一次批请求（输出 JSON 数组），
/// 对批响应里缺失的 idx 逐条回退到单请求。返回与输入同序的 (Issue, 解析结果)。
async fn refine_triage_batch(db: &Db, batch: Vec<Issue>) -> Vec<(Issue, Option<TriageParsed>)> {
    if batch.len() <= 1 {
        let mut out = Vec::with_capacity(batch.len());
        for issue in batch {
            let parsed = refine_triage_one(db, &issue).await;
            out.push((issue, parsed));
        }
        return out;
    }

    // 同批共用一次召回（按首条所在项目），系统提示词来自 triage 角色；
    // 这里用 user prompt 显式切到「批量数组」模式，覆盖单对象输出约束。
    let mut prompt = String::with_capacity(256);
    prompt.push_str(&format!(
        "本次为**批量整理模式**（忽略系统提示中「只输出一个对象」的约束）。下面是 {} 条待整理的原始碎片，每条以 [序号] 开头。\n\
         请对每条按同样的整理规则处理，输出一个**严格 JSON 数组**（不要 Markdown、不要解释文字），\n\
         数组每个元素字段：idx（整数，对应下面的序号）、title、category、severity、description、clarity_score、needs_clarification、missing_info、duplicate_of、is_noise（含义同单条规则）。\n\
         务必为每个序号都返回恰好一个元素，只输出 JSON 数组。\n",
        batch.len()
    ));
    for (i, it) in batch.iter().enumerate() {
        prompt.push_str(&format!("\n[{}] {}\n", i, triage_raw(it)));
    }

    let project_id = batch[0].project_id.clone();
    let (raw, trace_id) = crate::agents::llm::run_system_role_text_traced(
        db, "triage", &prompt, None, Some(&project_id), None,
    )
    .await
    .unwrap_or_else(|_| (String::new(), None));
    let map = parse_triage_batch(&raw);

    let mut results = Vec::with_capacity(batch.len());
    for (i, issue) in batch.into_iter().enumerate() {
        match map.get(&i) {
            Some(p) => {
                // 批量命中：用本批共享 trace_id 落库该 issue 的结构化产出。
                record_triage(db, &issue, p, "ok", trace_id.as_deref(), &raw).await;
                results.push((issue, Some(p.clone())));
            }
            // 批响应漏了这条 → 回退单条整理（其内部自行落库），绝不丢需求。
            None => {
                let parsed = refine_triage_one(db, &issue).await;
                results.push((issue, parsed));
            }
        }
    }
    results
}

/// 按「项目边界 + 条数/字符预算」把碎片切成小批，控制单次请求体量，
/// 避免输出过长被截断（token 感知的粗粒度近似：字符数 ≈ token 的保守上界）。
fn group_for_triage(items: Vec<Issue>) -> Vec<Vec<Issue>> {
    const MAX_ITEMS: usize = 8;
    const MAX_CHARS: usize = 6000;
    let mut batches: Vec<Vec<Issue>> = Vec::new();
    let mut cur: Vec<Issue> = Vec::new();
    let mut cur_chars = 0usize;
    for it in items {
        let c = triage_raw(&it).chars().count();
        let cross_project = cur
            .first()
            .map(|f: &Issue| f.project_id != it.project_id)
            .unwrap_or(false);
        if !cur.is_empty()
            && (cross_project || cur.len() >= MAX_ITEMS || cur_chars + c > MAX_CHARS)
        {
            batches.push(std::mem::take(&mut cur));
            cur_chars = 0;
        }
        cur_chars += c;
        cur.push(it);
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    batches
}

/// 批量整理一组碎片：切批 + 有界并发跑 triage Agent。
/// 返回与输入**对应**的 (Issue, 解析结果)；解析结果为 None 表示该条 LLM/解析失败。
/// 纯逻辑——不写库、不发事件，由调用方决定整理结果如何落地。
pub async fn batch_triage(db: &Db, issues: Vec<Issue>) -> Vec<(Issue, Option<TriageParsed>)> {
    const BATCH_CONCURRENCY: usize = 4;
    let batches = group_for_triage(issues);
    let nested: Vec<Vec<(Issue, Option<TriageParsed>)>> = futures::stream::iter(
        batches.into_iter().map(|batch| async move { refine_triage_batch(db, batch).await }),
    )
    .buffer_unordered(BATCH_CONCURRENCY)
    .collect()
    .await;
    nested.into_iter().flatten().collect()
}

/// **前置去噪**：对给定 id 中仍处于 triage 的碎片跑整理，噪音直接删除，
/// 有效碎片**就地**补全 title/category/severity/description 但**保留 `status='triage'`**——
/// 不自动进流水线（保住自喂料安全护栏 C4），只是把待整理池清洗干净，
/// 人工闸口看到的是已去噪、已归一化的条目。
///
/// 与命令层的 `refine_triage` 区别：后者把有效碎片转入 `pending_analysis` 并入队分析，
/// 是「人点头放行进流水线」；本函数只做清洗，是「自动滤噪、仍候人审」。
pub async fn denoise_in_place(db: &Db, issue_ids: Vec<String>) -> DenoiseStats {
    let mut stats = DenoiseStats::default();
    if issue_ids.is_empty() {
        return stats;
    }

    // 取出仍处于 triage 的碎片（跳过已不存在/状态已变的，幂等）。
    let mut loaded = Vec::new();
    for id in issue_ids {
        if let Ok(Some(issue)) =
            sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=? AND status='triage'")
                .bind(&id)
                .fetch_optional(db)
                .await
        {
            loaded.push(issue);
        }
    }
    if loaded.is_empty() {
        return stats;
    }

    for (issue, parsed) in batch_triage(db, loaded).await {
        let Some(p) = parsed else {
            stats.errors += 1;
            continue;
        };
        // 噪音、或 triage 判定为重复（duplicate_of 指向另一条已有需求）→ 一并丢弃。
        // duplicate_of 此前被解析出来却从未使用，是「频繁重复」漏网的一环：现在闭合。
        let is_duplicate = p
            .duplicate_of
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if p.is_noise || is_duplicate {
            let _ = sqlx::query("DELETE FROM issues WHERE id=? AND status='triage'")
                .bind(&issue.id)
                .execute(db)
                .await;
            stats.discarded += 1;
            continue;
        }
        // 就地归一化，保留 triage 状态（不进流水线）。
        let _ = sqlx::query(
            "UPDATE issues SET title=?, category=?, severity=?, description=?,
             updated_at=datetime('now') WHERE id=? AND status='triage'",
        )
        .bind(&p.title)
        .bind(&p.category)
        .bind(&p.severity)
        .bind(&p.description)
        .bind(&issue.id)
        .execute(db)
        .await;
        stats.normalized += 1;
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    // §3.5 防漂移：规范样例必须能解析进 struct 且关键字段非默认，
    // 改 struct/字段名而忘改 prompt（或反之）时此测试会失败。
    #[test]
    fn triage_item_parses_canonical_example() {
        let ex = r#"[{"idx":0,"title":"修复登录偶发失败","category":"Bug","severity":"high",
          "description":"登录在弱网下偶发失败","clarity_score":0.8,"needs_clarification":false,
          "missing_info":["复现步骤"],"duplicate_of":null,"is_noise":false}]"#;
        let (items, st) = schema::parse_array_or_empty::<TriageParsed>(ex);
        assert_eq!(st, "ok");
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.title, "修复登录偶发失败");
        assert_eq!(it.category, "Bug");
        assert_eq!(it.idx, Some(0));
        assert_eq!(it.clarity_score, Some(0.8));
        assert_eq!(it.missing_info.len(), 1);
    }

    #[test]
    fn validate_rejects_empty_non_noise_keeps_noise() {
        let empty = TriageParsed::default();
        assert!(validate(empty).is_none()); // title 空且非噪音 → 无效
        let noise = TriageParsed { is_noise: true, ..TriageParsed::default() };
        assert!(validate(noise).is_some()); // 噪音可无标题
    }

    #[test]
    fn default_keeps_category_severity() {
        let d = TriageParsed::default();
        assert_eq!(d.category, "Feature");
        assert_eq!(d.severity, "medium");
        assert_eq!(<TriageParsed as StructuredSchema>::ROLE, "triage");
    }
}
