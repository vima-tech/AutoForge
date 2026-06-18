use super::IntakePayload;
use std::path::Path;
use tokio::process::Command;
use tracing::warn;

/// 递归扫描源码文件，命中 TODO/FIXME/HACK/XXX 行。纯 Rust 实现，跨平台一致
/// （不再 shell-out `grep`——Windows 无 grep）。返回 `path:line:内容` 形式的行。
fn grep_todos(repo_path: &str) -> Vec<String> {
    const EXTS: &[&str] = &["rs", "ts", "tsx", "js", "py", "go"];
    const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "__pycache__", "dist", "build"];
    const TAGS: &[&str] = &["TODO", "FIXME", "HACK", "XXX"];

    fn looks_like_tag(line: &str) -> bool {
        TAGS.iter().any(|tag| {
            if let Some(idx) = line.find(tag) {
                // 紧跟 `:` 或 `(...):`，且其后有非空内容，近似原正则。
                let rest = line[idx + tag.len()..].trim_start();
                let rest = rest.strip_prefix('(').map_or(rest, |r| {
                    r.split_once(')').map_or(rest, |(_, after)| after)
                });
                rest.strip_prefix(':')
                    .map(|after| !after.trim().is_empty())
                    .unwrap_or(false)
            } else {
                false
            }
        })
    }

    let mut hits: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![Path::new(repo_path).to_path_buf()];
    while let Some(dir) = stack.pop() {
        if hits.len() >= 100 {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || SKIP_DIRS.contains(&name) {
                    continue;
                }
                stack.push(path);
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !EXTS.contains(&ext) {
                    continue;
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue, // 二进制/非 UTF-8 跳过
                };
                let rel = path.to_string_lossy();
                for (i, line) in content.lines().enumerate() {
                    if hits.len() >= 100 {
                        break;
                    }
                    if looks_like_tag(line) {
                        hits.push(format!("{}:{}:{}", rel, i + 1, line.trim()));
                    }
                }
            }
        }
    }
    hits
}

/// 扫描代码库中的 TODO/FIXME/HACK/XXX 注释
pub async fn scan_todos(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let repo_path_owned = repo_path.to_string();
    // 阻塞文件遍历放到 blocking 线程池，避免卡住 tokio 工作线程。
    let lines = match tokio::task::spawn_blocking(move || grep_todos(&repo_path_owned)).await {
        Ok(v) => v,
        Err(e) => {
            warn!("[scanner] todo scan failed: {}", e);
            return vec![];
        }
    };

    lines
        .iter()
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

    // Route through the platform shell: on Windows `npm` is `npm.cmd`, which
    // `Command::new("npm")` cannot resolve directly (CreateProcess only appends
    // `.exe`). `cmd /C` / `sh -lc` resolve the shim correctly.
    let output = crate::core::platform::shell("npm audit --json")
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

/// 运行 pip-audit（Python 依赖漏洞）。需仓库含 requirements.txt 或 pyproject.toml。
pub async fn scan_pip_audit(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    if !Path::new(repo_path).join("requirements.txt").exists()
        && !Path::new(repo_path).join("pyproject.toml").exists()
    {
        return vec![];
    }

    // pip-audit 输出 JSON：{"dependencies":[{"name","version","vulns":[{"id","description","fix_versions"}]}]}
    let output = crate::core::platform::shell("pip-audit -f json")
        .current_dir(repo_path)
        .output()
        .await;
    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => {
            warn!("[scanner] pip-audit failed: {}", e);
            return vec![];
        }
    };
    let val: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    // 兼容两种顶层形态：{"dependencies":[...]} 或直接数组。
    let deps = val
        .get("dependencies")
        .and_then(|d| d.as_array())
        .cloned()
        .or_else(|| val.as_array().cloned())
        .unwrap_or_default();

    let mut out = Vec::new();
    for dep in deps {
        let pkg = dep.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
        let version = dep.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let vulns = match dep.get("vulns").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };
        for vuln in vulns {
            if out.len() >= 50 {
                return out;
            }
            let id = vuln.get("id").and_then(|i| i.as_str()).unwrap_or("UNKNOWN");
            let desc = vuln
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let fix = vuln
                .get("fix_versions")
                .and_then(|f| f.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            out.push(IntakePayload {
                project_id: project_id.to_string(),
                title: format!("[安全] pip {}: {}", pkg, id),
                description: Some(format!(
                    "Advisory: {}\nPackage: {} {}\nFix: {}\n\n{}",
                    id, pkg, version, fix, desc
                )),
                category: Some("Debt".to_string()),
                // pip-audit 默认不给 CVSS 评级，统一标 medium，交由分析阶段定级。
                severity: Some("medium".to_string()),
                source_type: "security_audit".to_string(),
                source_ref: Some(format!("pip:{}", id)),
            });
        }
    }
    out
}

/// 运行 govulncheck（Go 依赖/调用链漏洞）。需仓库含 go.mod。
pub async fn scan_govulncheck(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    if !Path::new(repo_path).join("go.mod").exists() {
        return vec![];
    }

    // govulncheck -json 输出**串联的 JSON 对象流**（非单一数组），用流式反序列化逐个读，
    // 收集含 "osv" 字段的漏洞条目。
    let output = crate::core::platform::shell("govulncheck -json ./...")
        .current_dir(repo_path)
        .output()
        .await;
    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => {
            warn!("[scanner] govulncheck failed: {}", e);
            return vec![];
        }
    };

    let mut out = Vec::new();
    let stream = serde_json::Deserializer::from_str(&stdout).into_iter::<serde_json::Value>();
    for item in stream.flatten() {
        let Some(osv) = item.get("osv") else { continue };
        if out.len() >= 50 {
            break;
        }
        let id = osv.get("id").and_then(|i| i.as_str()).unwrap_or("UNKNOWN");
        let summary = osv
            .get("summary")
            .and_then(|s| s.as_str())
            .or_else(|| osv.get("details").and_then(|d| d.as_str()))
            .unwrap_or("");
        out.push(IntakePayload {
            project_id: project_id.to_string(),
            title: format!(
                "[安全] go {}: {}",
                id,
                summary.chars().take(80).collect::<String>()
            ),
            description: Some(format!("Advisory: {}\n\n{}", id, summary)),
            category: Some("Debt".to_string()),
            severity: Some("high".to_string()),
            source_type: "security_audit".to_string(),
            source_ref: Some(format!("go:{}", id)),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_todo_tags_recursively_cross_platform() {
        let dir = std::env::temp_dir().join(format!("af-scan-{}", std::process::id()));
        let sub = dir.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(sub.join("a.rs"), "// TODO: fix me\nlet x = 1;\n").unwrap();
        std::fs::write(sub.join("b.ts"), "// FIXME(bob): broken\n// note: plain\n").unwrap();
        std::fs::write(dir.join("node_modules").join("c.js"), "// TODO: ignored\n").unwrap();
        std::fs::write(sub.join("d.png"), "TODO: not source\n").unwrap();

        let hits = grep_todos(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).ok();

        assert!(hits.iter().any(|h| h.contains("TODO: fix me")));
        assert!(hits.iter().any(|h| h.contains("FIXME(bob): broken")));
        // skipped: node_modules, non-source ext, and "note:" (not a tag)
        assert!(!hits.iter().any(|h| h.contains("ignored")));
        assert!(!hits.iter().any(|h| h.contains("not source")));
        assert!(!hits.iter().any(|h| h.contains("plain")));
    }
}
