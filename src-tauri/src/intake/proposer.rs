//! 工厂自喂料：proposer Agent 基于项目现状主动提议改进，产物进 triage 池。
//!
//! 纯 Rust、零 Tauri 类型：本模块只**生成** `IntakePayload`，由调用方（命令/调度器）
//! 经 `gateway::receive(.., IntakeMode::Triage)` 落库——既复用去重+注入过滤，也保证
//! 自喂料**永远只进待整理池**、绝不自动进流水线（安全护栏见 C4）。

use anyhow::Result;

use crate::db::Db;
use crate::intake::IntakePayload;

/// 运行 proposer，返回至多 `max` 条提议载荷（source_type=proposer）。
/// 失败（未配置角色/LLM、解析失败）返回 Err 或空向量，由调用方静默处理。
pub async fn propose(db: &Db, project_id: &str, max: usize) -> Result<Vec<IntakePayload>> {
    let max = max.clamp(1, 20);
    let instruction = format!(
        "请基于本项目的代码现状与上下文，提出最多 {} 条最值得做的改进/需求。\
         工程类每条必须带 file:line 证据，宁缺毋滥。",
        max
    );
    let out = crate::agents::llm::run_system_role_text(
        db,
        "proposer",
        &instruction,
        None,
        Some(project_id),
        None,
    )
    .await?;

    let proposals = parse_proposals(&out);
    Ok(proposals
        .into_iter()
        .take(max)
        .map(|p| IntakePayload {
            project_id: project_id.to_string(),
            title: p.title,
            description: Some(if p.evidence.trim().is_empty() {
                p.description
            } else {
                format!("{}\n\n— 证据：{}", p.description, p.evidence)
            }),
            category: Some(p.category),
            severity: Some(p.severity),
            source_type: "proposer".to_string(),
            source_ref: None,
        })
        .collect())
}

struct Proposal {
    title: String,
    category: String,
    severity: String,
    evidence: String,
    description: String,
}

/// 解析 proposer 输出的 JSON 数组（容忍 ```json 围栏与前后噪声）。
fn parse_proposals(out: &str) -> Vec<Proposal> {
    let Some(start) = out.find('[') else { return vec![] };
    let Some(end) = out.rfind(']') else { return vec![] };
    if end <= start {
        return vec![];
    }
    let arr: serde_json::Value = match serde_json::from_str(&out[start..=end]) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let Some(items) = arr.as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|v| {
            let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(Proposal {
                title,
                category: v.get("category").and_then(|x| x.as_str()).unwrap_or("Improvement").to_string(),
                severity: v.get("severity").and_then(|x| x.as_str()).unwrap_or("medium").to_string(),
                evidence: v.get("evidence").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect()
}
