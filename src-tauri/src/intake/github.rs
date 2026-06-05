use super::IntakePayload;
use crate::core::security;
use tracing::{info, warn};

/// GitHub Issues 字段到 AutoForge 字段的映射
struct GithubIssue {
    number: i64,
    title: String,
    body: String,
    labels: Vec<String>,
}

/// 从 GitHub REST API 拉取所有 open Issues（分页）
pub async fn fetch_issues(
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<Vec<(i64, IntakePayload)>, String> {
    let client = reqwest::Client::builder()
        .user_agent("AutoForge/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut all: Vec<GithubIssue> = vec![];
    let mut page = 1u32;

    loop {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&per_page=100&page={}",
            owner, repo, page
        );
        let mut req = client.get(&url).header("Accept", "application/vnd.github.v3+json");
        if let Some(tok) = token {
            if !tok.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", tok));
            }
        }

        let resp = req.send().await.map_err(|e| format!("GitHub API 请求失败: {}", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub API 错误 {}: {}", status, body));
        }

        let items: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let arr = match items.as_array() {
            Some(a) => a.clone(),
            None => break,
        };
        if arr.is_empty() {
            break;
        }

        for item in &arr {
            // 跳过 Pull Request（GitHub API 将 PR 包含在 issues 端点）
            if item.get("pull_request").is_some() {
                continue;
            }
            let number = item.get("number").and_then(|n| n.as_i64()).unwrap_or(0);
            let title = item
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let body = item
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("")
                .to_string();
            let labels: Vec<String> = item
                .get("labels")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                        .map(|s| s.to_lowercase())
                        .collect()
                })
                .unwrap_or_default();

            if !title.is_empty() {
                all.push(GithubIssue { number, title, body, labels });
            }
        }

        if arr.len() < 100 {
            break;
        }
        page += 1;
    }

    info!("[github] fetched {} issues from {}/{}", all.len(), owner, repo);

    // 转换为 IntakePayload
    let project_id_placeholder = String::new(); // 由 command 层填入
    let result = all
        .into_iter()
        .map(|gh| {
            let category = map_labels_to_category(&gh.labels);
            let severity = map_labels_to_severity(&gh.labels);
            let payload = IntakePayload {
                project_id: project_id_placeholder.clone(),
                title: security::safe_truncate(&gh.title, 200),
                description: Some(security::safe_truncate(
                    if gh.body.is_empty() { &gh.title } else { &gh.body },
                    4000,
                )),
                category: Some(category),
                severity: Some(severity),
                source_type: "github".to_string(),
                source_ref: Some(format!("github:{}/{}#{}", owner, repo, gh.number)),
            };
            (gh.number, payload)
        })
        .collect();

    Ok(result)
}

fn map_labels_to_category(labels: &[String]) -> String {
    for label in labels {
        match label.as_str() {
            "bug" | "defect" | "error" => return "Bug".to_string(),
            "improvement" | "enhancement" => return "Improvement".to_string(),
            "debt" | "tech-debt" | "technical-debt" | "refactor" => return "Debt".to_string(),
            "feature" | "new feature" | "feat" => return "Feature".to_string(),
            _ => {}
        }
    }
    "Feature".to_string()
}

fn map_labels_to_severity(labels: &[String]) -> String {
    for label in labels {
        match label.as_str() {
            "critical" | "p0" | "blocker" => return "critical".to_string(),
            "high" | "p1" | "major" => return "high".to_string(),
            "medium" | "p2" | "normal" => return "medium".to_string(),
            "low" | "p3" | "minor" | "trivial" => return "low".to_string(),
            _ => {}
        }
    }
    "medium".to_string()
}
