//! 待整理池（triage）的整理引擎 —— **纯 Rust、零 Tauri 类型**。
//!
//! 把 triage Agent 的批量整理逻辑从 `commands/intake.rs` 下沉到这里，使得
//! 它既能被人工触发的 `refine_triage` 命令复用，也能被工厂自喂料
//! (`tasks/autosupply.rs`) 在**入池后立即前置去噪**——噪音不必等人点「整理」
//! 才被滤掉（见 [`denoise_in_place`]）。
//!
//! 解耦铁律：本模块只依赖 `Db` 与 `Issue`，不触碰 `AppHandle`/`State`/事件发射。
//! 命令层负责在整理结果上做「进流水线 + 发事件」这类带 Tauri 类型的动作。

use crate::db::Db;
use crate::models::issue::Issue;
use futures::StreamExt;
use std::collections::HashMap;

/// 一条碎片整理后的归一化结果。
#[derive(Clone)]
pub struct TriageParsed {
    pub title: String,
    pub category: String,
    pub severity: String,
    pub description: String,
    /// triage Agent 判定该碎片为噪音/无价值，应直接丢弃。
    pub is_noise: bool,
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

/// 从单个 JSON 对象抽取 triage 字段（容错缺省）。title 为空且非噪音视为无效。
fn triage_from_value(v: &serde_json::Value) -> Option<TriageParsed> {
    let is_noise = v.get("is_noise").and_then(|x| x.as_bool()).unwrap_or(false);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if title.is_empty() && !is_noise {
        return None;
    }
    Some(TriageParsed {
        title,
        category: v.get("category").and_then(|x| x.as_str()).unwrap_or("Feature").to_string(),
        severity: v.get("severity").and_then(|x| x.as_str()).unwrap_or("medium").to_string(),
        description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        is_noise,
    })
}

/// 解析 triage Agent 输出的单个 JSON 对象（容忍 ```json 围栏与前后噪声）。
fn parse_triage_json(out: &str) -> Option<TriageParsed> {
    let start = out.find('{')?;
    let end = out.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out[start..=end]).ok()?;
    triage_from_value(&v)
}

/// 解析批量 triage 输出的 JSON 数组：返回 `idx -> TriageParsed` 映射。
/// 每个元素需带 `idx`（或 `index`/`id`）整数指向输入序号；缺失/坏元素被跳过，
/// 由上层对未命中的 idx 回退到单条整理，保证不丢需求。
fn parse_triage_batch(out: &str) -> HashMap<usize, TriageParsed> {
    let mut map = HashMap::new();
    let (Some(start), Some(end)) = (out.find('['), out.rfind(']')) else {
        return map;
    };
    if end <= start {
        return map;
    }
    let Ok(serde_json::Value::Array(arr)) =
        serde_json::from_str::<serde_json::Value>(&out[start..=end])
    else {
        return map;
    };
    for el in &arr {
        let idx = el
            .get("idx")
            .or_else(|| el.get("index"))
            .or_else(|| el.get("id"))
            .and_then(|x| x.as_u64());
        if let (Some(idx), Some(parsed)) = (idx, triage_from_value(el)) {
            map.insert(idx as usize, parsed);
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

/// 单条整理：调 triage Agent → 解析单 JSON 对象。失败返回 None（计为出错）。
async fn refine_triage_one(db: &Db, issue: &Issue) -> Option<TriageParsed> {
    crate::agents::llm::run_system_role_text(
        db,
        "triage",
        &triage_raw(issue),
        None,
        Some(&issue.project_id),
        None,
    )
    .await
    .ok()
    .and_then(|out| parse_triage_json(&out))
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
         数组每个元素字段：idx（整数，对应下面的序号）、title、category、severity、description、is_noise（含义同单条规则）。\n\
         务必为每个序号都返回恰好一个元素，只输出 JSON 数组。\n",
        batch.len()
    ));
    for (i, it) in batch.iter().enumerate() {
        prompt.push_str(&format!("\n[{}] {}\n", i, triage_raw(it)));
    }

    let project_id = batch[0].project_id.clone();
    let map = match crate::agents::llm::run_system_role_text(
        db, "triage", &prompt, None, Some(&project_id), None,
    )
    .await
    {
        Ok(out) => parse_triage_batch(&out),
        Err(_) => HashMap::new(),
    };

    let mut results = Vec::with_capacity(batch.len());
    for (i, issue) in batch.into_iter().enumerate() {
        match map.get(&i) {
            Some(p) => results.push((issue, Some(p.clone()))),
            // 批响应漏了这条 → 回退单条整理，绝不丢需求。
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
        if p.is_noise {
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
