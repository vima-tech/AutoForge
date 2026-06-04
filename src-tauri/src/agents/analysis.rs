use super::local_claude;
use crate::core::security;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const SYSTEM_PROMPT: &str = r#"你是 AutoForge 的需求分析 Agent。
请严格按照以下 JSON 格式输出分析结果，不要输出任何其他内容：
{
  "authenticity_score": <0.0-1.0 真实性评分>,
  "feasibility_score": <0.0-1.0 可行性评分>,
  "priority_suggestion": <1-10 优先级>,
  "category_suggestion": "<Feature|Bug|Debt|Improvement>",
  "severity_suggestion": "<low|medium|high|critical>",
  "affected_modules": ["<模块1>", "<模块2>"],
  "analysis_summary": "<50字以内的摘要>",
  "is_duplicate": <true|false>,
  "duplicate_hint": "<如果疑似重复，简述原因，否则为空字符串>"
}
评估标准：
- authenticity_score: 需求是否真实、明确、有业务价值
- feasibility_score: 技术上是否可行，结合项目上下文中的技术栈和现有模块判断
- priority_suggestion: 综合业务价值和紧迫性
- affected_modules: 结合项目目录结构和代码库判断受影响的具体模块
- is_duplicate: 是否与已有需求重复（保守判断）
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub authenticity_score: f64,
    pub feasibility_score: f64,
    pub priority_suggestion: i64,
    pub category_suggestion: String,
    pub severity_suggestion: String,
    pub affected_modules: Vec<String>,
    pub analysis_summary: String,
    pub is_duplicate: bool,
    pub duplicate_hint: String,
    pub raw_output: String,
}

impl Default for AnalysisResult {
    fn default() -> Self {
        Self {
            authenticity_score: 0.5,
            feasibility_score: 0.5,
            priority_suggestion: 5,
            category_suggestion: "Feature".to_string(),
            severity_suggestion: "medium".to_string(),
            affected_modules: vec![],
            analysis_summary: "自动分析失败，使用默认值".to_string(),
            is_duplicate: false,
            duplicate_hint: String::new(),
            raw_output: String::new(),
        }
    }
}

#[allow(dead_code)]
pub fn check_safety(text: &str) -> bool {
    !security::has_obvious_injection(text)
}

/// Read key files from a local repository to build a project context string
/// that enriches the analysis prompt. Gracefully degrades if any read fails.
pub async fn build_project_context(repo_path: &str) -> String {
    use tokio::fs;

    let mut parts: Vec<String> = Vec::new();

    // README
    for name in &["README.md", "README.zh.md", "README.en.md", "README.txt", "README"] {
        let path = format!("{}/{}", repo_path, name);
        if let Ok(content) = fs::read_to_string(&path).await {
            let trimmed: String = content.chars().take(3000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("## {}\n{}", name, trimmed));
                break;
            }
        }
    }

    // Primary manifest/config file
    for name in &["package.json", "Cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle"] {
        let path = format!("{}/{}", repo_path, name);
        if let Ok(content) = fs::read_to_string(&path).await {
            let trimmed: String = content.chars().take(1000).collect();
            if !trimmed.trim().is_empty() {
                parts.push(format!("## {}\n```\n{}\n```", name, trimmed));
                break;
            }
        }
    }

    // Directory tree (depth 2, skip hidden / build dirs)
    if let Ok(out) = Command::new("find")
        .arg(repo_path)
        .arg("-maxdepth").arg("2")
        .arg("-type").arg("d")
        .arg("!").arg("-path").arg("*/.*")
        .arg("!").arg("-path").arg("*/node_modules/*")
        .arg("!").arg("-path").arg("*/target/*")
        .arg("!").arg("-path").arg("*/__pycache__/*")
        .output()
        .await
    {
        let tree = String::from_utf8_lossy(&out.stdout);
        let limited: String = tree.chars().take(1500).collect();
        if !limited.trim().is_empty() {
            parts.push(format!("## 目录结构\n{}", limited));
        }
    }

    // Recent git log
    if let Ok(out) = Command::new("git")
        .arg("-C").arg(repo_path)
        .arg("log")
        .arg("--oneline")
        .arg("-15")
        .output()
        .await
    {
        let log = String::from_utf8_lossy(&out.stdout);
        if !log.trim().is_empty() {
            parts.push(format!("## 近期提交记录\n{}", log));
        }
    }

    parts.join("\n\n")
}

pub async fn analyze(
    title: &str,
    description: &str,
    project_context: Option<&str>,
) -> Result<AnalysisResult> {
    let prompt = match project_context.filter(|c| !c.trim().is_empty()) {
        Some(ctx) => format!(
            "以下是项目的上下文信息，请结合项目实际情况进行分析：\n\n{}\n\n---\n\n请分析以下需求：\n\n标题：{}\n\n描述：{}\n\n请按照要求的 JSON 格式输出分析结果。",
            ctx, title, description
        ),
        None => format!(
            "请分析以下需求：\n\n标题：{}\n\n描述：{}\n\n请按照要求的 JSON 格式输出分析结果。",
            title, description
        ),
    };

    let raw = local_claude::run_text(&prompt, Some(SYSTEM_PROMPT)).await?;

    // Try to parse JSON from the output
    let parsed = parse_analysis_json(&raw);
    let mut result = parsed.unwrap_or_default();
    result.raw_output = raw;
    Ok(result)
}

fn parse_analysis_json(text: &str) -> Option<AnalysisResult> {
    // Find JSON block in output
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if start > end {
        return None;
    }
    let json_str = &text[start..=end];

    #[derive(Deserialize)]
    struct Raw {
        authenticity_score: f64,
        feasibility_score: Option<f64>,
        priority_suggestion: Option<i64>,
        category_suggestion: Option<String>,
        severity_suggestion: Option<String>,
        affected_modules: Option<Vec<String>>,
        analysis_summary: Option<String>,
        is_duplicate: Option<bool>,
        duplicate_hint: Option<String>,
    }

    let r: Raw = serde_json::from_str(json_str).ok()?;
    Some(AnalysisResult {
        authenticity_score: r.authenticity_score.clamp(0.0, 1.0),
        feasibility_score: r.feasibility_score.unwrap_or(0.5).clamp(0.0, 1.0),
        priority_suggestion: r.priority_suggestion.unwrap_or(5).clamp(1, 10),
        category_suggestion: r
            .category_suggestion
            .unwrap_or_else(|| "Feature".to_string()),
        severity_suggestion: r
            .severity_suggestion
            .unwrap_or_else(|| "medium".to_string()),
        affected_modules: r.affected_modules.unwrap_or_default(),
        analysis_summary: r.analysis_summary.unwrap_or_default(),
        is_duplicate: r.is_duplicate.unwrap_or(false),
        duplicate_hint: r.duplicate_hint.unwrap_or_default(),
        raw_output: String::new(),
    })
}
