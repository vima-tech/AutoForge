use super::IntakePayload;
use std::path::Path;
use tokio::process::Command;
use tracing::warn;

/// 扫描代码库中的 TODO/FIXME/HACK/XXX 注释
pub async fn scan_todos(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let output = Command::new("grep")
        .args([
            "-rn",
            "--include=*.rs",
            "--include=*.ts",
            "--include=*.tsx",
            "--include=*.js",
            "--include=*.py",
            "--include=*.go",
            "-E",
            r"(TODO|FIXME|HACK|XXX)(\([^)]*\))?:\s*.+",
            repo_path,
        ])
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            warn!("[scanner] grep failed: {}", e);
            return vec![];
        }
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .take(100)
        .filter_map(|line| {
            // 格式：path/to/file.rs:123:// TODO: fix this
            let mut parts = line.splitn(3, ':');
            let file_path = parts.next()?;
            let line_num = parts.next()?;
            let comment = parts.next()?.trim();

            let todo_text = comment
                .trim_start_matches("//")
                .trim_start_matches("/*")
                .trim_start_matches('#')
                .trim()
                .to_string();

            let title: String = todo_text.chars().take(120).collect();
            if title.is_empty() {
                return None;
            }

            Some(IntakePayload {
                project_id: project_id.to_string(),
                title: format!("[扫描] {}", title),
                description: Some(format!(
                    "代码注释：{}\n位置：{} 第{}行",
                    todo_text, file_path, line_num
                )),
                category: Some("Debt".to_string()),
                severity: Some("low".to_string()),
                source_type: "todo_scan".to_string(),
                source_ref: Some(format!("todo:{}:{}", file_path, line_num)),
            })
        })
        .collect()
}

/// 运行 cargo audit，解析安全漏洞
pub async fn scan_cargo_audit(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    if !Path::new(repo_path).join("Cargo.lock").exists() {
        return vec![];
    }

    let output = Command::new("cargo")
        .args(["audit", "--json"])
        .current_dir(repo_path)
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            warn!("[scanner] cargo audit failed: {}", e);
            return vec![];
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let vulns = match val
        .get("vulnerabilities")
        .and_then(|v| v.get("list"))
        .and_then(|l| l.as_array())
    {
        Some(a) => a.clone(),
        None => return vec![],
    };

    vulns
        .into_iter()
        .take(50)
        .filter_map(|vuln| {
            let advisory = vuln.get("advisory")?;
            let advisory_id = advisory.get("id")?.as_str()?.to_string();
            let title = advisory.get("title")?.as_str()?.to_string();
            let pkg = vuln
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let severity = advisory
                .get("cvss")
                .and_then(|c| c.get("score"))
                .and_then(|s| s.as_f64())
                .map(|score| {
                    if score >= 9.0 {
                        "critical"
                    } else if score >= 7.0 {
                        "high"
                    } else if score >= 4.0 {
                        "medium"
                    } else {
                        "low"
                    }
                })
                .unwrap_or("medium");

            Some(IntakePayload {
                project_id: project_id.to_string(),
                title: format!("[安全] {}: {}", pkg, title),
                description: Some(format!(
                    "Advisory: {}\nPackage: {}\nSeverity: {}",
                    advisory_id, pkg, severity
                )),
                category: Some("Debt".to_string()),
                severity: Some(severity.to_string()),
                source_type: "security_audit".to_string(),
                source_ref: Some(format!("advisory:{}", advisory_id)),
            })
        })
        .collect()
}

/// 运行 npm audit，解析安全漏洞
pub async fn scan_npm_audit(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    if !Path::new(repo_path).join("package-lock.json").exists()
        && !Path::new(repo_path).join("yarn.lock").exists()
    {
        return vec![];
    }

    let output = Command::new("npm")
        .args(["audit", "--json"])
        .current_dir(repo_path)
        .output()
        .await;

    // npm audit exits non-zero when vulnerabilities exist; stdout still has JSON
    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => {
            warn!("[scanner] npm audit failed: {}", e);
            return vec![];
        }
    };

    let val: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let vulns = match val.get("vulnerabilities").and_then(|v| v.as_object()) {
        Some(m) => m.clone(),
        None => return vec![],
    };

    vulns
        .into_iter()
        .take(50)
        .filter_map(|(pkg_name, vuln)| {
            let severity = vuln.get("severity")?.as_str()?.to_string();
            let via = vuln.get("via")?.as_array()?;
            let desc: String = via
                .iter()
                .filter_map(|v| {
                    if v.is_string() {
                        v.as_str().map(|s| s.to_string())
                    } else {
                        v.get("title")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");

            Some(IntakePayload {
                project_id: project_id.to_string(),
                title: format!(
                    "[安全] npm {}: {}",
                    pkg_name,
                    desc.chars().take(80).collect::<String>()
                ),
                description: Some(format!(
                    "Package: {}\nSeverity: {}\nVia: {}",
                    pkg_name, severity, desc
                )),
                category: Some("Debt".to_string()),
                severity: Some(severity),
                source_type: "security_audit".to_string(),
                source_ref: Some(format!("npm:{}", pkg_name)),
            })
        })
        .collect()
}
