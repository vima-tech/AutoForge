use super::IntakePayload;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use tracing::warn;

/// 递归扫描源码文件，命中 TODO/FIXME/HACK/XXX 行。纯 Rust 实现，跨平台一致
/// （不再 shell-out `grep`——Windows 无 grep）。返回 `path:line:内容` 形式的行。
fn grep_todos(repo_path: &str) -> Vec<String> {
    const EXTS: &[&str] = &["rs", "ts", "tsx", "js", "py", "go"];
    const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "__pycache__", "dist", "build"];
    // 高信号标签：明确标记「待修/隐患/危险」，价值远高于泛泛的 TODO。
    const HIGH_SIGNAL_TAGS: &[&str] = &["FIXME", "HACK", "XXX", "BUG", "SAFETY"];
    // 普通 TODO 仅当行内含下列风险/安全关键词时才收（其余 TODO 视为低价值噪音丢弃）。
    const TODO_KEYWORDS: &[&str] = &[
        "security", "vuln", "inject", "race", "deadlock", "leak", "overflow", "panic",
        "unsafe", "unwrap", "auth", "crash", "data loss", "corrupt", "dos",
        "安全", "漏洞", "注入", "竞态", "死锁", "泄漏", "泄露", "溢出", "崩溃", "鉴权", "越权",
    ];

    // 返回该行是否命中、且应被收录（高信号标签直接收；TODO 需含风险关键词）。
    fn looks_like_tag(line: &str) -> bool {
        // 标签后须紧跟 `:` 或 `(...):` 且其后有非空内容，近似原正则。
        fn tagged(line: &str, tag: &str) -> bool {
            if let Some(idx) = line.find(tag) {
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
        }
        if HIGH_SIGNAL_TAGS.iter().any(|tag| tagged(line, tag)) {
            return true;
        }
        if tagged(line, "TODO") {
            let lower = line.to_lowercase();
            return TODO_KEYWORDS.iter().any(|kw| lower.contains(kw));
        }
        false
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
            // 格式：file_path:line:comment，例如 src/x.rs:123 处的一条标记注释
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
                // 已过高信号筛选（FIXME/HACK/XXX/BUG/SAFETY 或含风险关键词的 TODO），按 medium 计。
                severity: Some("medium".to_string()),
                source_type: "todo_scan".to_string(),
                source_ref: Some(format!("todo:{}:{}", file_path, line_num)),
            })
        })
        .collect()
}

/// 一条「半成品/未实现」命中。
struct Unfinished {
    file: String,
    line: usize,
    text: String,
    severity: &'static str,
    category: &'static str,
    note: &'static str,
}

/// 半成品/未实现桩扫描：揪出运行时会崩或明显未完成的功能点——`todo!()`/`unimplemented!()`、
/// `NotImplementedError`、`panic("not implemented")`、以及注释里的「待/未实现」。这是 linter 与
/// 依赖审计都覆盖不到、却最能反映「功能只做了一半」的强信号（直击「很多功能半进度」的痛点）。
/// 纯 Rust 行扫描，跨平台一致；只收极低误报的强信号模式。
fn grep_unfinished(repo_path: &str) -> Vec<Unfinished> {
    use regex::Regex;
    const EXTS: &[&str] = &["rs", "ts", "tsx", "js", "py", "go"];
    const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "__pycache__", "dist", "build"];

    // 运行时会 panic 的 Rust 未实现桩：近零误报。
    let rust_stub = Regex::new(r"\b(todo!|unimplemented!)\s*\(").ok();
    // 显式「未实现」错误（跨语言）：NotImplementedError / not implemented / not yet implemented。
    let not_impl = Regex::new(r"(?i)not[\s_]*(yet[\s_]*)?implement").ok();
    // 注释里的中文「待/未/尚未/暂未 实现」——仅当该行确为注释时计，避免误伤关键词数组等字符串。
    let zh_marker = Regex::new(r"(待|未|尚未|暂未|暂不)\s*实现").ok();

    fn is_comment_line(line: &str) -> bool {
        let t = line.trim_start();
        t.starts_with("//") || t.starts_with('#') || t.starts_with("/*") || t.starts_with('*')
    }

    let mut hits: Vec<Unfinished> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![Path::new(repo_path).to_path_buf()];
    while let Some(dir) = stack.pop() {
        if hits.len() >= 80 {
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
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !EXTS.contains(&ext) {
                continue;
            }
            // 跳过本扫描器自身——它的模式定义行会自我命中，纯噪音。
            let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if base == "scanner.rs" {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let rel = path.to_string_lossy().to_string();
            for (i, line) in content.lines().enumerate() {
                if hits.len() >= 80 {
                    break;
                }
                if line.len() > 400 {
                    continue; // 超长行多为生成物/压缩代码，跳过。
                }
                let (severity, category, note) = if rust_stub.as_ref().is_some_and(|r| r.is_match(line)) {
                    ("high", "Bug", "运行时会 panic 的未实现桩（todo!/unimplemented!）")
                } else if not_impl.as_ref().is_some_and(|r| r.is_match(line)) {
                    ("high", "Bug", "显式标记为未实现（not implemented）")
                } else if is_comment_line(line) && zh_marker.as_ref().is_some_and(|r| r.is_match(line)) {
                    ("medium", "Debt", "注释标记功能待实现")
                } else {
                    continue;
                };
                hits.push(Unfinished {
                    file: rel.clone(),
                    line: i + 1,
                    text: line.trim().chars().take(160).collect(),
                    severity,
                    category,
                    note,
                });
            }
        }
    }
    hits
}

/// 扫描半成品/未实现功能点，产出 `source_type=unfinished_scan` 的待整理项。
pub async fn scan_unfinished(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let repo = repo_path.to_string();
    let hits = match tokio::task::spawn_blocking(move || grep_unfinished(&repo)).await {
        Ok(v) => v,
        Err(e) => {
            warn!("[scanner] unfinished scan failed: {}", e);
            return vec![];
        }
    };
    let pid = project_id.to_string();
    hits.into_iter()
        .map(|h| {
            let title: String = format!("[半成品] {}", h.note);
            IntakePayload {
                project_id: pid.clone(),
                title,
                description: Some(format!(
                    "{}\n\n位置：{}:{}\n代码：{}",
                    h.note, h.file, h.line, h.text
                )),
                category: Some(h.category.to_string()),
                severity: Some(h.severity.to_string()),
                source_type: "unfinished_scan".to_string(),
                source_ref: Some(format!("unfinished:{}:{}", h.file, h.line)),
            }
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

// ── 静态代码分析（发现真实代码问题，区别于只查依赖漏洞的 *_audit）────────────────
//
// TODO/FIXME 扫描只能发现「人主动标注」的待办；依赖审计只查第三方漏洞。
// 真正的代码缺陷（未使用变量、可疑写法、类型错误、可疑比较……）要靠各语言的
// 静态分析器。这里按 `core::stack` 检测到的栈，调度对应分析器，把告警转成 intake。
//
// 设计：best-effort——分析器未安装/超时/无配置一律静默返回空，绝不阻塞自喂料；
// 解析逻辑抽成纯函数（带单测），异步壳只负责跑命令 + 计时保护。

/// 单个分析器最长运行时间（编译型如 clippy 可能较久）。
const ANALYZER_TIMEOUT_SECS: u64 = 300;
/// 单个分析器最多产出的告警条数（防一次性淹没）。
const ANALYZER_MAX_FINDINGS: usize = 50;

/// 跑一条命令并加超时保护；超时/启动失败返回 None。`kill_on_drop` 保证超时后进程被回收。
async fn run_capped(mut cmd: Command, secs: u64) -> Option<std::process::Output> {
    cmd.kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(secs), cmd.output()).await {
        Ok(Ok(o)) => Some(o),
        Ok(Err(e)) => {
            warn!("[analyzer] command failed: {}", e);
            None
        }
        Err(_) => {
            warn!("[analyzer] command timed out after {}s", secs);
            None
        }
    }
}

/// 按栈画像调度静态分析器，汇总所有告警为 intake 载荷。
pub async fn scan_static_analysis(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let analyzers = crate::core::stack::code_analyzers(Path::new(repo_path));
    let mut out = Vec::new();
    for a in analyzers {
        let mut found = match a {
            "clippy" => scan_clippy(project_id, repo_path).await,
            "ruff" => scan_ruff(project_id, repo_path).await,
            "go_vet" => scan_go_vet(project_id, repo_path).await,
            "eslint" => scan_eslint(project_id, repo_path).await,
            _ => vec![],
        };
        out.append(&mut found);
    }
    out
}

/// clippy（Rust）：`cargo clippy --message-format=json`。Tauri 仓库自动定位 src-tauri 清单。
pub async fn scan_clippy(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let root = Path::new(repo_path);
    let manifest = if root.join("src-tauri/Cargo.toml").exists() {
        Some("src-tauri/Cargo.toml")
    } else if root.join("Cargo.toml").exists() {
        Some("Cargo.toml")
    } else {
        return vec![];
    };
    let mut cmd = Command::new("cargo");
    cmd.args(["clippy", "--message-format=json", "--quiet"]);
    if let Some(m) = manifest {
        cmd.args(["--manifest-path", m]);
    }
    cmd.current_dir(repo_path);
    let Some(output) = run_capped(cmd, ANALYZER_TIMEOUT_SECS).await else {
        return vec![];
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_clippy_json(project_id, &stdout)
}

/// 高价值 clippy 警告白名单（perf / suspicious / 并发 / 正确性相关）。
/// clippy 的 correctness 组本就 deny-by-default（level=error）会无条件收录；这里补收
/// 一批**虽为 warning 但确有价值**的 lint，其余 style/complexity/pedantic/nursery 噪音一律丢弃。
fn is_high_value_clippy(code: &str) -> bool {
    const ALLOW: &[&str] = &[
        // 并发隐患（持锁 await → 易死锁）
        "await_holding_lock",
        "await_holding_refcell_ref",
        "await_holding_invalid_type",
        // 可疑/疑似 bug
        "float_cmp",
        "eq_op",
        "logic_bug",
        "nonsensical_open_options",
        "suspicious_operation_groupings",
        "mem_replace_with_uninit",
        "invalid_regex",
        "drop_non_drop",
        "forget_non_drop",
        // 性能
        "redundant_clone",
        "needless_collect",
        "or_fun_call",
        "large_enum_variant",
        "box_collection",
        "vec_box",
        "unnecessary_to_owned",
        "inefficient_to_string",
        "manual_memcpy",
        "large_stack_arrays",
        "boxed_local",
        // 健壮性
        "unwrap_used",
        "expect_used",
        "indexing_slicing",
        "panic_in_result_fn",
        "lossy_float_literal",
    ];
    let bare = code.rsplit("::").next().unwrap_or(code);
    ALLOW.contains(&bare)
}

/// 解析 clippy `--message-format=json` 的串联 JSON 行流（每行一个对象）。
fn parse_clippy_json(project_id: &str, stdout: &str) -> Vec<IntakePayload> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        if out.len() >= ANALYZER_MAX_FINDINGS {
            break;
        }
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("");
        if level != "warning" && level != "error" {
            continue;
        }
        // 取主 span 的 file:line；无 span（汇总行如「aborting due to…」）跳过。
        let span = msg
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|arr| arr.iter().find(|s| s.get("is_primary").and_then(|p| p.as_bool()).unwrap_or(false)).or_else(|| arr.first()));
        let Some(span) = span else { continue };
        let file = span.get("file_name").and_then(|f| f.as_str()).unwrap_or("");
        let line_no = span.get("line_start").and_then(|l| l.as_u64()).unwrap_or(0);
        if file.is_empty() {
            continue;
        }
        let code = msg
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or("clippy");
        // 降噪：error（correctness，deny-by-default）无条件收；warning 仅收高价值白名单，
        // 其余 style/complexity/pedantic/nursery 一律丢弃，避免「皮毛」条目淹没待整理池。
        if level != "error" && !is_high_value_clippy(code) {
            continue;
        }
        let text = msg.get("message").and_then(|m| m.as_str()).unwrap_or("");
        let short: String = text.chars().take(80).collect();
        out.push(IntakePayload {
            project_id: project_id.to_string(),
            title: format!("[分析] clippy {}: {}", code, short),
            description: Some(format!(
                "规则：{}\n位置：{}:{}\n级别：{}\n\n{}",
                code, file, line_no, level, text
            )),
            category: Some(if level == "error" { "Bug" } else { "Debt" }.to_string()),
            // error→high；幸存的高价值 warning→medium（不再是 low，过得了严重度门槛）。
            severity: Some(if level == "error" { "high" } else { "medium" }.to_string()),
            source_type: "code_analysis".to_string(),
            source_ref: Some(format!("clippy:{}:{}", file, line_no)),
        });
    }
    out
}

/// ruff（Python）：`ruff check --output-format json .`。
pub async fn scan_ruff(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let mut cmd = Command::new("ruff");
    cmd.args(["check", "--output-format", "json", "."]);
    cmd.current_dir(repo_path);
    let Some(output) = run_capped(cmd, ANALYZER_TIMEOUT_SECS).await else {
        return vec![];
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ruff_json(project_id, &stdout)
}

/// 解析 ruff JSON 数组：`[{filename, location:{row}, code, message}]`。
fn parse_ruff_json(project_id: &str, stdout: &str) -> Vec<IntakePayload> {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(stdout.trim())
    else {
        return vec![];
    };
    arr.iter()
        .take(ANALYZER_MAX_FINDINGS)
        .filter_map(|v| {
            let file = v.get("filename").and_then(|f| f.as_str()).unwrap_or("");
            if file.is_empty() {
                return None;
            }
            let line_no = v
                .get("location")
                .and_then(|l| l.get("row"))
                .and_then(|r| r.as_u64())
                .unwrap_or(0);
            let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("ruff");
            let text = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
            let short: String = text.chars().take(80).collect();
            Some(IntakePayload {
                project_id: project_id.to_string(),
                title: format!("[分析] ruff {}: {}", code, short),
                description: Some(format!("规则：{}\n位置：{}:{}\n\n{}", code, file, line_no, text)),
                category: Some("Debt".to_string()),
                severity: Some("low".to_string()),
                source_type: "code_analysis".to_string(),
                source_ref: Some(format!("ruff:{}:{}", file, line_no)),
            })
        })
        .collect()
}

/// go vet（Go）：诊断写在 stderr，形如 `path:line:col: message`。
pub async fn scan_go_vet(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let mut cmd = Command::new("go");
    cmd.args(["vet", "./..."]);
    cmd.current_dir(repo_path);
    let Some(output) = run_capped(cmd, ANALYZER_TIMEOUT_SECS).await else {
        return vec![];
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_go_vet(project_id, &stderr)
}

/// 解析 go vet stderr：取形如 `file:line:col: msg` 或 `file:line: msg` 的诊断行。
fn parse_go_vet(project_id: &str, stderr: &str) -> Vec<IntakePayload> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        if out.len() >= ANALYZER_MAX_FINDINGS {
            break;
        }
        let line = line.trim();
        // 跳过非诊断行（如 "# package/path" 标题、go 工具进度）。
        if line.is_empty() || line.starts_with('#') || line.starts_with("go:") {
            continue;
        }
        // 形如 file:line[:col]: message —— 至少含 "file:line: "。
        let mut parts = line.splitn(4, ':');
        let (Some(file), Some(line_no)) = (parts.next(), parts.next()) else {
            continue;
        };
        if file.is_empty() || line_no.parse::<u32>().is_err() {
            continue;
        }
        // 第三段可能是 col 或直接是消息；拼回剩余作为消息。
        let rest: Vec<&str> = [parts.next(), parts.next()].into_iter().flatten().collect();
        let text = rest.join(":");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let short: String = text.chars().take(80).collect();
        out.push(IntakePayload {
            project_id: project_id.to_string(),
            title: format!("[分析] go vet: {}", short),
            description: Some(format!("位置：{}:{}\n\n{}", file, line_no, text)),
            category: Some("Bug".to_string()),
            severity: Some("medium".to_string()),
            source_type: "code_analysis".to_string(),
            source_ref: Some(format!("go_vet:{}:{}", file, line_no)),
        });
    }
    out
}

/// eslint（JS/TS）：仅当仓库本地装了 eslint（`node_modules/.bin/eslint`）且有配置时运行，
/// 避免 `npx` 触发联网安装。通过平台 shell 跑本地 bin，输出 JSON。
pub async fn scan_eslint(project_id: &str, repo_path: &str) -> Vec<IntakePayload> {
    let root = Path::new(repo_path);
    if !root.join("node_modules/.bin/eslint").exists() {
        return vec![];
    }
    // 至少存在一种 eslint 配置（flat 或传统），否则跳过。
    let has_config = ["eslint.config.js", "eslint.config.mjs", "eslint.config.cjs", ".eslintrc",
        ".eslintrc.js", ".eslintrc.cjs", ".eslintrc.json", ".eslintrc.yml", ".eslintrc.yaml"]
        .iter()
        .any(|c| root.join(c).exists())
        || std::fs::read_to_string(root.join("package.json"))
            .map(|s| s.contains("\"eslintConfig\""))
            .unwrap_or(false);
    if !has_config {
        return vec![];
    }
    let mut cmd = crate::core::platform::shell("node_modules/.bin/eslint -f json .");
    cmd.current_dir(repo_path);
    let Some(output) = run_capped(cmd, ANALYZER_TIMEOUT_SECS).await else {
        return vec![];
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_eslint_json(project_id, &stdout)
}

/// 解析 eslint `-f json` 输出：`[{filePath, messages:[{line, ruleId, message, severity}]}]`。
/// severity 2=error→Bug/medium，1=warning→Debt/low，0(off) 跳过。
fn parse_eslint_json(project_id: &str, stdout: &str) -> Vec<IntakePayload> {
    let start = match stdout.find('[') {
        Some(s) => s,
        None => return vec![],
    };
    let end = match stdout.rfind(']') {
        Some(e) => e,
        None => return vec![],
    };
    if end <= start {
        return vec![];
    }
    let Ok(serde_json::Value::Array(files)) =
        serde_json::from_str::<serde_json::Value>(&stdout[start..=end])
    else {
        return vec![];
    };
    let mut out = Vec::new();
    for file_entry in &files {
        let file = file_entry.get("filePath").and_then(|f| f.as_str()).unwrap_or("");
        let Some(messages) = file_entry.get("messages").and_then(|m| m.as_array()) else {
            continue;
        };
        for m in messages {
            if out.len() >= ANALYZER_MAX_FINDINGS {
                return out;
            }
            let sev = m.get("severity").and_then(|s| s.as_u64()).unwrap_or(0);
            if sev == 0 {
                continue;
            }
            let line_no = m.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let rule = m.get("ruleId").and_then(|r| r.as_str()).unwrap_or("eslint");
            let text = m.get("message").and_then(|t| t.as_str()).unwrap_or("");
            let short: String = text.chars().take(80).collect();
            out.push(IntakePayload {
                project_id: project_id.to_string(),
                title: format!("[分析] eslint {}: {}", rule, short),
                description: Some(format!("规则：{}\n位置：{}:{}\n\n{}", rule, file, line_no, text)),
                category: Some(if sev >= 2 { "Bug" } else { "Debt" }.to_string()),
                severity: Some(if sev >= 2 { "medium" } else { "low" }.to_string()),
                source_type: "code_analysis".to_string(),
                source_ref: Some(format!("eslint:{}:{}", file, line_no)),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_todo_tags_recursively_cross_platform() {
        // 标签字面量用 concat! 在编译期拼出，避免本测试源码自身被 TODO 扫描器
        // （grep_todos）误当成待办命中、反复供料成需求。
        let todo = concat!("TO", "DO");
        let fixme = concat!("FIX", "ME");

        let dir = std::env::temp_dir().join(format!("af-scan-{}", std::process::id()));
        let sub = dir.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        // 普通 TODO 无风险关键词 → 降噪丢弃；含关键词（unsafe）的 TODO → 保留。
        std::fs::write(sub.join("a.rs"), format!("// {todo}: tidy up\n// {todo}: unsafe pointer cast\n")).unwrap();
        std::fs::write(sub.join("b.ts"), format!("// {fixme}(bob): broken\n// note: plain\n"))
            .unwrap();
        std::fs::write(dir.join("node_modules").join("c.js"), format!("// {todo}: ignored\n"))
            .unwrap();
        std::fs::write(sub.join("d.png"), format!("{todo}: not source\n")).unwrap();

        let hits = grep_todos(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).ok();

        // 高信号 FIXME 保留；含风险关键词的 TODO 保留。
        assert!(hits.iter().any(|h| h.contains(&format!("{fixme}(bob): broken"))));
        assert!(hits.iter().any(|h| h.contains("unsafe pointer cast")));
        // 丢弃：普通无关键词 TODO、node_modules、非源码、"note:"（非标签）。
        assert!(!hits.iter().any(|h| h.contains("tidy up")));
        assert!(!hits.iter().any(|h| h.contains("ignored")));
        assert!(!hits.iter().any(|h| h.contains("not source")));
        assert!(!hits.iter().any(|h| h.contains("plain")));
    }

    #[test]
    fn scans_unfinished_stubs_high_signal_only() {
        let dir = std::env::temp_dir().join(format!("af-unfin-{}", std::process::id()));
        let sub = dir.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        // 运行时桩（high/Bug）+ 显式 not implemented（high）+ 注释中文待实现（medium）。
        std::fs::write(
            sub.join("a.rs"),
            "fn f() { todo!(\"wire this\") }\nfn g() -> i32 { 1 + 1 }\n// 这里待实现：导出功能\n",
        )
        .unwrap();
        std::fs::write(
            sub.join("b.py"),
            "def h():\n    raise NotImplementedError\n",
        )
        .unwrap();
        // 字符串里出现“未实现”但不是注释行 → 不应命中（避免误伤关键词数组）。
        std::fs::write(sub.join("c.ts"), "const label = \"未实现的占位\";\n").unwrap();

        let hits = grep_unfinished(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).ok();

        assert!(hits.iter().any(|h| h.text.contains("todo!") && h.severity == "high"));
        assert!(hits.iter().any(|h| h.text.contains("NotImplementedError") && h.severity == "high"));
        assert!(hits.iter().any(|h| h.note.contains("注释标记") && h.severity == "medium"));
        // 正常函数体与字符串字面量不应命中。
        assert!(!hits.iter().any(|h| h.text.contains("1 + 1")));
        assert!(!hits.iter().any(|h| h.text.contains("const label")));
    }

    #[test]
    fn parses_clippy_findings_filters_noise_and_skips_summary() {
        // 行1：低价值 style warning（needless_return）→ 被降噪丢弃。
        // 行2：高价值 perf warning（redundant_clone）→ 保留为 medium。
        // 行4：error（无 code）→ 保留为 high；行5：无 span 汇总行 → 跳过。
        let stdout = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unneeded return statement","code":{"code":"clippy::needless_return"},"spans":[{"file_name":"src/a.rs","line_start":12,"is_primary":true}]}}
{"reason":"compiler-message","message":{"level":"warning","message":"redundant clone","code":{"code":"clippy::redundant_clone"},"spans":[{"file_name":"src/c.rs","line_start":7,"is_primary":true}]}}
{"reason":"compiler-artifact","package_id":"foo"}
{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","code":null,"spans":[{"file_name":"src/b.rs","line_start":3,"is_primary":true}]}}
{"reason":"compiler-message","message":{"level":"error","message":"aborting due to previous error","spans":[]}}"#;
        let out = parse_clippy_json("p1", stdout);
        assert_eq!(out.len(), 2, "low-value warning + artifact + summary(no span) skipped");
        // 高价值 warning 保留，severity=medium。
        assert!(out[0].title.contains("clippy clippy::redundant_clone"));
        assert_eq!(out[0].severity.as_deref(), Some("medium"));
        assert_eq!(out[0].category.as_deref(), Some("Debt"));
        // 无 code 时回退到 "clippy"；error → Bug/high。
        assert!(out[1].title.contains("clippy clippy"));
        assert_eq!(out[1].severity.as_deref(), Some("high"));
        assert_eq!(out[1].category.as_deref(), Some("Bug"));
        assert_eq!(out[1].source_ref.as_deref(), Some("clippy:src/b.rs:3"));
    }

    #[test]
    fn parses_ruff_json() {
        let stdout = r#"[{"filename":"app/main.py","location":{"row":7,"column":1},"code":"F401","message":"`os` imported but unused"}]"#;
        let out = parse_ruff_json("p1", stdout);
        assert_eq!(out.len(), 1);
        assert!(out[0].title.contains("ruff F401"));
        assert_eq!(out[0].source_ref.as_deref(), Some("ruff:app/main.py:7"));
        // 空/非数组输出安全返回空。
        assert!(parse_ruff_json("p1", "").is_empty());
        assert!(parse_ruff_json("p1", "not json").is_empty());
    }

    #[test]
    fn parses_go_vet_stderr() {
        let stderr = "# example.com/m\nmain.go:10:2: result of fmt.Sprintf call not used\nok\ngo: downloading x";
        let out = parse_go_vet("p1", stderr);
        assert_eq!(out.len(), 1, "only the file:line:col diagnostic counts");
        assert!(out[0].title.contains("go vet"));
        assert!(out[0].description.as_ref().unwrap().contains("fmt.Sprintf"));
        assert_eq!(out[0].source_ref.as_deref(), Some("go_vet:main.go:10"));
    }

    #[test]
    fn parses_eslint_json_skips_off() {
        let stdout = r#"[{"filePath":"/r/src/a.ts","messages":[{"line":4,"ruleId":"no-unused-vars","message":"'y' is defined but never used.","severity":1},{"line":9,"ruleId":"eqeqeq","message":"Expected '===' and instead saw '=='.","severity":2},{"line":1,"ruleId":null,"message":"off","severity":0}]}]"#;
        let out = parse_eslint_json("p1", stdout);
        assert_eq!(out.len(), 2, "severity 0 skipped");
        assert_eq!(out[0].severity.as_deref(), Some("low"));
        assert_eq!(out[0].category.as_deref(), Some("Debt"));
        assert_eq!(out[1].severity.as_deref(), Some("medium"));
        assert_eq!(out[1].category.as_deref(), Some("Bug"));
        assert!(out[1].title.contains("eslint eqeqeq"));
    }
}
