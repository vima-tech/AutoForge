//! 栈画像（Stack Profile）注册表 —— 让 AutoForge 适配多种项目类型。
//!
//! 目标：从一个仓库目录嗅出它由哪些技术栈组成，并给出**常规默认命令**
//! （dev / build / test / lint / security）、需要软链复用的**依赖缓存目录**，
//! 以及适用的**安全扫描器**。覆盖三类项目：
//!   - 桌面应用（Tauri）
//!   - Web 系统（前端框架 + Java/Go/Python/Node 后端）
//!   - 企业网站（前端框架 / 纯静态站）
//!
//! 设计约束（见 CLAUDE.md「后端独立化」铁律）：本模块是**纯 Rust、零 Tauri**，
//! 只做文件嗅探与字符串模板，不引用任何 `tauri::*` / `AppState` / DB 类型。
//! 端口位置统一用 `{port}` 占位（消费方 `dev_server::inject_port` 负责替换）。
//!
//! 这里给出的命令是**有依据的默认猜测**，不是保证——`run_config` 仍会让 AI
//! 结合仓库内容确认/修正，最终由人工审阅保存。

use std::path::Path;

/// 一个技术栈在项目中的角色。一个 Web 系统通常同时含 Frontend + Backend。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackRole {
    /// 前端框架（React/Vue/Next/Nuxt/Angular…），预览走 dev server + iframe。
    Frontend,
    /// 后端服务（Spring/Gin/FastAPI/Express…），预览靠端口可达性探活。
    Backend,
    /// 纯静态站（仅 index.html，无构建工具）。
    Static,
    /// 桌面应用（Tauri）。
    Desktop,
}

impl StackRole {
    fn as_str(self) -> &'static str {
        match self {
            StackRole::Frontend => "frontend",
            StackRole::Backend => "backend",
            StackRole::Static => "static",
            StackRole::Desktop => "desktop",
        }
    }
}

/// 一个被检测到的技术栈，附带常规默认命令。所有命令字段可空（拿不准则留空）。
#[derive(Debug, Clone)]
pub struct DetectedStack {
    /// 稳定标识，如 `java-maven` / `go` / `node-vite` / `tauri`。
    pub id: String,
    pub role: StackRole,
    /// 语言，如 `java` / `go` / `python` / `typescript` / `rust`。
    pub language: String,
    /// 框架（尽力而为，可能为空），如 `spring-boot` / `gin` / `fastapi` / `vue` / `react`。
    pub framework: String,
    /// 开发/预览启动命令（`{port}` 占位）。
    pub dev_command: Option<String>,
    /// 桌面应用启动命令（仅 Tauri）。
    pub app_command: Option<String>,
    /// 生产构建命令。
    pub build_command: Option<String>,
    /// 部署/预览启动命令（构建产物启动）。
    pub start_command: Option<String>,
    pub test_unit: Option<String>,
    pub test_integration: Option<String>,
    pub lint: Option<String>,
    pub typing: Option<String>,
    /// 安全扫描命令（如 `cargo audit` / `pip-audit` / `govulncheck ./...`）。
    pub security: Option<String>,
    /// 需软链进 worktree 以免重复安装的依赖目录（相对仓库根）。
    /// Java/Go/Python 用全局缓存（~/.m2、GOMODCACHE、pip cache），不在仓库内，故为空。
    pub dep_cache_dirs: Vec<String>,
    /// 适用的内置安全扫描器标识（供 intake 扫描调度）。
    pub scanners: Vec<&'static str>,
    /// 适用的内置**静态代码分析器**标识（clippy/ruff/go_vet/eslint…），
    /// 供自喂料/扫描发现真实代码问题——区别于 `scanners`（只查依赖漏洞）。
    pub analyzers: Vec<&'static str>,
}

impl DetectedStack {
    fn base(id: &str, role: StackRole, language: &str) -> Self {
        DetectedStack {
            id: id.to_string(),
            role,
            language: language.to_string(),
            framework: String::new(),
            dev_command: None,
            app_command: None,
            build_command: None,
            start_command: None,
            test_unit: None,
            test_integration: None,
            lint: None,
            typing: None,
            security: None,
            dep_cache_dirs: Vec::new(),
            scanners: Vec::new(),
            analyzers: Vec::new(),
        }
    }
}

fn exists(dir: &Path, rel: &str) -> bool {
    dir.join(rel).exists()
}

fn read_head(dir: &Path, rel: &str, max: usize) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(rel)).ok()?;
    Some(text.chars().take(max).collect())
}

/// 嗅出仓库使用的包管理器（前端/Node 栈）。lockfile 优先，回退 npm。
fn package_manager(dir: &Path) -> &'static str {
    if exists(dir, "pnpm-lock.yaml") {
        "pnpm"
    } else if exists(dir, "yarn.lock") {
        "yarn"
    } else if exists(dir, "bun.lockb") {
        "bun"
    } else {
        "npm"
    }
}

/// 读取 package.json 的 scripts 与 dependencies/devDependencies 合并键集。
fn read_pkg(dir: &Path) -> Option<(serde_json::Map<String, serde_json::Value>, Vec<String>)> {
    let raw = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let scripts = json
        .get("scripts")
        .and_then(|s| s.as_object())
        .cloned()
        .unwrap_or_default();
    let mut deps = Vec::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(obj) = json.get(key).and_then(|d| d.as_object()) {
            deps.extend(obj.keys().cloned());
        }
    }
    Some((scripts, deps))
}

/// `<pm> run <script>`（npm/bun 需要 run；pnpm/yarn 也接受 run，统一带上以免歧义）。
fn pm_run(pm: &str, script: &str) -> String {
    format!("{pm} run {script}")
}

/// 从 scripts 里挑第一个存在的脚本名。
fn first_script(scripts: &serde_json::Map<String, serde_json::Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|n| scripts.contains_key(**n))
        .map(|n| n.to_string())
}

/// 从 deps 键集判定前端框架名（按优先级，先命中先返回）。
fn frontend_framework(deps: &[String]) -> &'static str {
    let has = |needle: &str| deps.iter().any(|d| d == needle);
    if has("next") {
        "next"
    } else if has("nuxt") {
        "nuxt"
    } else if has("@angular/core") {
        "angular"
    } else if has("vue") {
        "vue"
    } else if has("svelte") || has("@sveltejs/kit") {
        "svelte"
    } else if has("react") {
        "react"
    } else {
        ""
    }
}

/// 是否像个 Node 后端（含 web 框架且无前端框架）。
fn is_node_backend(deps: &[String], fw: &str) -> bool {
    if !fw.is_empty() {
        return false;
    }
    let has = |needle: &str| deps.iter().any(|d| d == needle);
    has("express") || has("fastify") || has("koa") || has("@nestjs/core") || has("hapi")
}

/// 检测前端 / Node 栈（基于 package.json）。可能返回前端或 Node 后端栈。
fn detect_node(dir: &Path) -> Option<DetectedStack> {
    let (scripts, deps) = read_pkg(dir)?;
    let pm = package_manager(dir);
    let fw = frontend_framework(&deps);

    let build = first_script(&scripts, &["build"]).map(|s| pm_run(pm, &s));
    let test = first_script(&scripts, &["test", "test:unit", "vitest", "jest"]).map(|s| pm_run(pm, &s));
    let lint = first_script(&scripts, &["lint"]).map(|s| pm_run(pm, &s));
    let typing = first_script(&scripts, &["typecheck", "type-check", "tsc"]).map(|s| pm_run(pm, &s));

    if is_node_backend(&deps, fw) {
        let dev = first_script(&scripts, &["dev", "start", "serve"]).map(|s| pm_run(pm, &s));
        let mut s = DetectedStack::base("node-backend", StackRole::Backend, "typescript");
        s.framework = node_backend_framework(&deps).to_string();
        s.dev_command = dev;
        s.build_command = build;
        s.start_command = first_script(&scripts, &["start"]).map(|st| pm_run(pm, &st));
        s.test_unit = test;
        s.lint = lint;
        s.typing = typing;
        s.security = Some(format!("{pm} audit")).filter(|_| pm == "npm");
        s.dep_cache_dirs = vec!["node_modules".to_string()];
        s.scanners = vec!["npm_audit"];
        s.analyzers = vec!["eslint"];
        return Some(s);
    }

    // 前端框架（含通用 vite/无框架但有 package.json 的前端工程）。
    let dev = first_script(&scripts, &["dev", "serve", "start"]).map(|s| pm_run(pm, &s));
    let mut s = DetectedStack::base("node-frontend", StackRole::Frontend, "typescript");
    s.framework = fw.to_string();
    s.dev_command = dev;
    s.build_command = build;
    s.start_command = first_script(&scripts, &["preview", "start", "serve"]).map(|st| pm_run(pm, &st));
    s.test_unit = test;
    s.lint = lint;
    s.typing = typing;
    s.security = if pm == "npm" { Some("npm audit".to_string()) } else { None };
    s.dep_cache_dirs = vec!["node_modules".to_string()];
    s.scanners = vec!["npm_audit"];
    s.analyzers = vec!["eslint"];
    Some(s)
}

fn node_backend_framework(deps: &[String]) -> &'static str {
    let has = |needle: &str| deps.iter().any(|d| d == needle);
    if has("@nestjs/core") {
        "nestjs"
    } else if has("fastify") {
        "fastify"
    } else if has("koa") {
        "koa"
    } else if has("express") {
        "express"
    } else {
        ""
    }
}

/// 检测 Tauri 桌面应用（在 detect_node 结果上叠加 app_command）。
fn detect_tauri(dir: &Path) -> Option<DetectedStack> {
    if !(exists(dir, "src-tauri/tauri.conf.json") || exists(dir, "src-tauri/Cargo.toml")) {
        return None;
    }
    let pm = package_manager(dir);
    let (scripts, _deps) = read_pkg(dir).unwrap_or_default();
    // 优先脚本体里跑 `tauri dev` 的脚本，其次常见名。
    let tauri_script = scripts
        .iter()
        .find(|(_, v)| {
            let body = v.as_str().unwrap_or("");
            body.contains("tauri") && body.contains("dev")
        })
        .map(|(k, _)| k.clone())
        .or_else(|| first_script(&scripts, &["tauri:dev", "tauri-dev"]));
    let dev_script = if scripts.contains_key("dev") {
        Some("dev".to_string())
    } else {
        scripts
            .iter()
            .find(|(_, v)| {
                let body = v.as_str().unwrap_or("");
                (body.contains("vite") || body.contains("dev")) && !body.contains("tauri")
            })
            .map(|(k, _)| k.clone())
    };

    let mut s = DetectedStack::base("tauri", StackRole::Desktop, "rust+typescript");
    s.framework = "tauri".to_string();
    s.dev_command = dev_script.map(|sc| pm_run(pm, &sc));
    s.app_command = tauri_script.map(|sc| pm_run(pm, &sc));
    s.build_command = Some("cargo build --manifest-path src-tauri/Cargo.toml".to_string());
    s.test_unit = Some("cargo test --manifest-path src-tauri/Cargo.toml".to_string());
    s.lint = Some("cargo clippy --manifest-path src-tauri/Cargo.toml".to_string());
    // --no-fetch：合并安全门用本地缓存的 advisory-db，不在每次合并时实时拉取——避免上游
    // 新发布（后又撤回/收窄）的 advisory 非确定性地卡死合并；忽略项集中放仓库 .cargo/audit.toml。
    // advisory-db 的更新由周期巡检 scanner 负责（它会 fetch 并把新漏洞登记为需求）。
    s.security = Some("cargo audit --no-fetch --file src-tauri/Cargo.lock".to_string());
    // 软链 node_modules 免重装；软链 src-tauri/target 复用主仓库的编译产物——
    // 否则每个分支预览 worktree 都要从零全量编译 Rust（数分钟），这是「拉起很慢」主因。
    // cargo 对 target 目录有文件锁，并发构建会串行而非损坏，安全。
    s.dep_cache_dirs = vec!["node_modules".to_string(), "src-tauri/target".to_string()];
    s.scanners = vec!["npm_audit", "cargo_audit"];
    s.analyzers = vec!["clippy", "eslint"];
    Some(s)
}

/// 检测独立 Rust 后端/库（非 Tauri）。
fn detect_rust(dir: &Path) -> Option<DetectedStack> {
    if !exists(dir, "Cargo.toml") || exists(dir, "src-tauri/Cargo.toml") {
        return None;
    }
    let mut s = DetectedStack::base("rust", StackRole::Backend, "rust");
    s.dev_command = Some("cargo run".to_string());
    s.build_command = Some("cargo build --release".to_string());
    s.test_unit = Some("cargo test".to_string());
    s.lint = Some("cargo clippy --all-targets".to_string());
    // --no-fetch：合并门用缓存 advisory-db，避免实时拉取导致的非确定性阻断（见 tauri 分支注释）。
    s.security = Some("cargo audit --no-fetch".to_string());
    // 软链 target 复用主仓库编译缓存，避免分支预览每次冷构建（同 tauri 理由）。
    s.dep_cache_dirs = vec!["target".to_string()];
    s.scanners = vec!["cargo_audit"];
    s.analyzers = vec!["clippy"];
    Some(s)
}

/// 检测 Java（Maven / Gradle）。框架尽力识别 Spring Boot。
fn detect_java(dir: &Path) -> Option<DetectedStack> {
    let is_maven = exists(dir, "pom.xml");
    let is_gradle =
        exists(dir, "build.gradle") || exists(dir, "build.gradle.kts") || exists(dir, "settings.gradle");
    if !is_maven && !is_gradle {
        return None;
    }
    let build_text = read_head(dir, "pom.xml", 4000)
        .or_else(|| read_head(dir, "build.gradle", 4000))
        .or_else(|| read_head(dir, "build.gradle.kts", 4000))
        .unwrap_or_default();
    let is_spring = build_text.contains("spring-boot") || build_text.contains("springframework.boot");

    if is_maven {
        let mut s = DetectedStack::base("java-maven", StackRole::Backend, "java");
        if is_spring {
            s.framework = "spring-boot".to_string();
            s.dev_command = Some(
                "mvn -q spring-boot:run -Dspring-boot.run.arguments=--server.port={port}".to_string(),
            );
        }
        s.build_command = Some("mvn -q -DskipTests package".to_string());
        s.test_unit = Some("mvn -q test".to_string());
        s.test_integration = Some("mvn -q verify".to_string());
        s.security = Some("mvn -q org.owasp:dependency-check-maven:check".to_string());
        s.scanners = vec![];
        return Some(s);
    }

    // Gradle
    let gw = if exists(dir, "gradlew") { "./gradlew" } else { "gradle" };
    let mut s = DetectedStack::base("java-gradle", StackRole::Backend, "java");
    if is_spring {
        s.framework = "spring-boot".to_string();
        s.dev_command = Some(format!("{gw} bootRun --args='--server.port={{port}}'"));
    }
    s.build_command = Some(format!("{gw} build -x test"));
    s.test_unit = Some(format!("{gw} test"));
    s.security = Some(format!("{gw} dependencyCheckAnalyze"));
    s.scanners = vec![];
    Some(s)
}

/// 检测 Go。框架尽力识别 gin/echo/fiber。
fn detect_go(dir: &Path) -> Option<DetectedStack> {
    if !exists(dir, "go.mod") {
        return None;
    }
    let gomod = read_head(dir, "go.mod", 8000).unwrap_or_default();
    let framework = if gomod.contains("gin-gonic/gin") {
        "gin"
    } else if gomod.contains("labstack/echo") {
        "echo"
    } else if gomod.contains("gofiber/fiber") {
        "fiber"
    } else {
        ""
    };
    let mut s = DetectedStack::base("go", StackRole::Backend, "go");
    s.framework = framework.to_string();
    // Go 端口惯例走 PORT 环境变量（无统一 flag），故 dev 命令不带 {port}。
    s.dev_command = Some("go run .".to_string());
    s.build_command = Some("go build ./...".to_string());
    s.test_unit = Some("go test ./...".to_string());
    s.lint = Some("go vet ./...".to_string());
    s.security = Some("govulncheck ./...".to_string());
    s.scanners = vec!["govulncheck"];
    s.analyzers = vec!["go_vet"];
    Some(s)
}

/// 检测 Python（poetry / pip / django）。框架尽力识别 django/fastapi/flask。
fn detect_python(dir: &Path) -> Option<DetectedStack> {
    let is_poetry = exists(dir, "pyproject.toml");
    let is_pip = exists(dir, "requirements.txt");
    let is_django = exists(dir, "manage.py");
    if !is_poetry && !is_pip && !is_django {
        return None;
    }
    let dep_text = read_head(dir, "pyproject.toml", 6000)
        .unwrap_or_default()
        + &read_head(dir, "requirements.txt", 6000).unwrap_or_default();
    let dep_lc = dep_text.to_lowercase();
    let framework = if is_django || dep_lc.contains("django") {
        "django"
    } else if dep_lc.contains("fastapi") {
        "fastapi"
    } else if dep_lc.contains("flask") {
        "flask"
    } else {
        ""
    };
    // poetry 项目命令前缀 `poetry run`，否则裸命令。
    let run = if is_poetry { "poetry run " } else { "" };

    let mut s = DetectedStack::base(
        if is_poetry { "python-poetry" } else { "python-pip" },
        StackRole::Backend,
        "python",
    );
    s.framework = framework.to_string();
    s.dev_command = match framework {
        "django" => Some(format!("{run}python manage.py runserver 0.0.0.0:{{port}}")),
        "fastapi" => Some(format!("{run}uvicorn main:app --reload --port {{port}}")),
        "flask" => Some(format!("{run}flask run --port {{port}}")),
        _ => None,
    };
    s.test_unit = Some(format!("{run}pytest"));
    s.lint = Some(format!("{run}ruff check ."));
    s.security = Some("pip-audit".to_string());
    s.scanners = vec!["pip_audit"];
    s.analyzers = vec!["ruff"];
    Some(s)
}

/// 检测纯静态站（有 index.html、无 package.json/构建工具）。
fn detect_static(dir: &Path) -> Option<DetectedStack> {
    if !exists(dir, "index.html") || exists(dir, "package.json") {
        return None;
    }
    let mut s = DetectedStack::base("static", StackRole::Static, "html");
    s.dev_command = Some("python3 -m http.server {port}".to_string());
    Some(s)
}

/// 检测一个目录里的所有技术栈，**按优先级排序**（桌面 > 后端 > 前端 > 静态）。
/// 一个 Web 系统会同时返回后端栈与前端栈。
pub fn detect_stacks(dir: &Path) -> Vec<DetectedStack> {
    let mut out: Vec<DetectedStack> = Vec::new();

    // Tauri 与普通前端二选一（Tauri 已含前端 + app_command）。
    if let Some(t) = detect_tauri(dir) {
        out.push(t);
    } else if let Some(n) = detect_node(dir) {
        out.push(n);
    }

    // 后端栈（可与前端共存，构成 Web 系统）。
    for s in [detect_rust(dir), detect_java(dir), detect_go(dir), detect_python(dir)]
        .into_iter()
        .flatten()
    {
        out.push(s);
    }

    // 纯静态站兜底（仅当啥都没检测到时）。
    if out.is_empty() {
        if let Some(s) = detect_static(dir) {
            out.push(s);
        }
    }

    // 排序：Desktop > Backend > Frontend > Static，保证 primary 取到合理代表。
    fn rank(r: StackRole) -> u8 {
        match r {
            StackRole::Desktop => 0,
            StackRole::Backend => 1,
            StackRole::Frontend => 2,
            StackRole::Static => 3,
        }
    }
    out.sort_by_key(|s| rank(s.role));
    out
}

/// 需软链进 worktree 的依赖缓存目录（去重、且实际存在于仓库内）。
pub fn dep_cache_dirs(dir: &Path) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for s in detect_stacks(dir) {
        for d in s.dep_cache_dirs {
            if !seen.contains(&d) && dir.join(&d).exists() {
                seen.push(d);
            }
        }
    }
    seen
}

/// 适用于该仓库的内置**静态代码分析器**标识（去重）——clippy/ruff/go_vet/eslint。
/// 供自喂料/扫描发现真实代码问题（区别于只查依赖漏洞的安全扫描器）。
pub fn code_analyzers(dir: &Path) -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    for s in detect_stacks(dir) {
        for a in s.analyzers {
            if !seen.contains(&a) {
                seen.push(a);
            }
        }
    }
    seen
}

/// 运行配置建议——中性结构（不依赖 commands 层的 RunConfigDraft），
/// 由 `run_config` 映射到前端表单字段。
#[derive(Debug, Default, Clone)]
pub struct RunConfigSuggestion {
    pub dev_kind: Option<String>,
    pub dev_command: Option<String>,
    pub app_command: Option<String>,
    pub build_command: Option<String>,
    pub start_command: Option<String>,
    pub test_unit: Option<String>,
    pub test_integration: Option<String>,
    pub lint: Option<String>,
    pub typing: Option<String>,
    pub security: Option<String>,
    pub language: Option<String>,
    pub framework: Option<String>,
    /// 检测到的栈一句话摘要，供注入 AI 提示。
    pub summary: String,
}

fn opt(s: Option<String>) -> Option<String> {
    s.filter(|x| !x.trim().is_empty())
}

/// 根据检测到的栈合成一份运行配置建议。
///
/// 规则：
/// - dev/app/kind：取「前端或桌面」栈（预览面向用户界面）。无前端时退回后端 dev。
/// - test/build/lint/security：后端优先（核心质量闸口），无后端则用前端。
/// - language/framework：取优先级最高的栈（桌面>后端>前端>静态）。
pub fn suggest_run_config(dir: &Path) -> RunConfigSuggestion {
    let stacks = detect_stacks(dir);
    if stacks.is_empty() {
        return RunConfigSuggestion {
            summary: "未识别到已知技术栈".to_string(),
            ..Default::default()
        };
    }

    let primary = &stacks[0];
    let ui = stacks
        .iter()
        .find(|s| matches!(s.role, StackRole::Frontend | StackRole::Desktop | StackRole::Static));
    let backend = stacks.iter().find(|s| s.role == StackRole::Backend);

    let dev_kind = match ui.map(|s| s.role) {
        Some(StackRole::Desktop) => Some("tauri".to_string()),
        Some(_) => Some("web".to_string()),
        None => backend.map(|_| "web".to_string()),
    };

    // dev/app 来自 UI 栈；纯后端项目用后端 dev。
    let (dev_command, app_command) = match ui {
        Some(u) => (u.dev_command.clone(), u.app_command.clone()),
        None => (backend.and_then(|b| b.dev_command.clone()), None),
    };

    // 质量类命令后端优先。
    let qsrc = backend.or(Some(primary)).unwrap();
    let build_command = backend
        .and_then(|b| b.build_command.clone())
        .or_else(|| ui.and_then(|u| u.build_command.clone()));
    let start_command = backend
        .and_then(|b| b.start_command.clone())
        .or_else(|| ui.and_then(|u| u.start_command.clone()));

    let summary = stacks
        .iter()
        .map(|s| {
            let fw = if s.framework.is_empty() {
                String::new()
            } else {
                format!("/{}", s.framework)
            };
            format!("{}({}{})", s.id, s.role.as_str(), fw)
        })
        .collect::<Vec<_>>()
        .join(" + ");

    RunConfigSuggestion {
        dev_kind,
        dev_command: opt(dev_command),
        app_command: opt(app_command),
        build_command: opt(build_command),
        start_command: opt(start_command),
        test_unit: opt(qsrc.test_unit.clone()),
        test_integration: opt(qsrc.test_integration.clone()),
        lint: opt(qsrc.lint.clone().or_else(|| ui.and_then(|u| u.lint.clone()))),
        typing: opt(qsrc.typing.clone().or_else(|| ui.and_then(|u| u.typing.clone()))),
        security: opt(qsrc.security.clone()),
        language: Some(primary.language.clone()),
        framework: opt(Some(primary.framework.clone())),
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("af-stack-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn detects_spring_boot_maven() {
        let d = tmp("maven");
        write(&d, "pom.xml", "<project><dependency>spring-boot-starter-web</dependency></project>");
        let stacks = detect_stacks(&d);
        let java = stacks.iter().find(|s| s.id == "java-maven").unwrap();
        assert_eq!(java.language, "java");
        assert_eq!(java.framework, "spring-boot");
        assert!(java.dev_command.as_ref().unwrap().contains("{port}"));
        assert_eq!(java.test_unit.as_deref(), Some("mvn -q test"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_go_gin() {
        let d = tmp("go");
        write(&d, "go.mod", "module x\nrequire github.com/gin-gonic/gin v1.9.0\n");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.id, "go");
        assert_eq!(s.framework, "gin");
        assert_eq!(s.test_unit.as_deref(), Some("go test ./..."));
        assert_eq!(s.security.as_deref(), Some("govulncheck ./..."));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_python_fastapi() {
        let d = tmp("py");
        write(&d, "requirements.txt", "fastapi==0.110\nuvicorn\n");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.id, "python-pip");
        assert_eq!(s.framework, "fastapi");
        assert!(s.dev_command.as_ref().unwrap().contains("--port {port}"));
        assert_eq!(s.security.as_deref(), Some("pip-audit"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_django() {
        let d = tmp("dj");
        write(&d, "manage.py", "# django");
        write(&d, "requirements.txt", "Django==5.0\n");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.framework, "django");
        assert!(s.dev_command.as_ref().unwrap().contains("runserver 0.0.0.0:{port}"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_vue_frontend() {
        let d = tmp("vue");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"dev":"vite","build":"vite build","test":"vitest","lint":"eslint ."},"dependencies":{"vue":"^3.4"}}"#,
        );
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.role, StackRole::Frontend);
        assert_eq!(s.framework, "vue");
        assert_eq!(s.dev_command.as_deref(), Some("npm run dev"));
        assert_eq!(s.dep_cache_dirs, vec!["node_modules".to_string()]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_node_express_backend() {
        let d = tmp("express");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"dev":"nodemon","start":"node index.js"},"dependencies":{"express":"^4"}}"#,
        );
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.role, StackRole::Backend);
        assert_eq!(s.id, "node-backend");
        assert_eq!(s.framework, "express");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn pnpm_lockfile_switches_package_manager() {
        let d = tmp("pnpm");
        write(&d, "pnpm-lock.yaml", "lockfileVersion: 6.0\n");
        write(&d, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"react":"^18"}}"#);
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.dev_command.as_deref(), Some("pnpm run dev"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn web_system_detects_frontend_and_backend() {
        let d = tmp("websys");
        write(&d, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"react":"^18"}}"#);
        write(&d, "go.mod", "module x\nrequire github.com/gin-gonic/gin v1\n");
        let stacks = detect_stacks(&d);
        assert!(stacks.iter().any(|s| s.role == StackRole::Backend));
        assert!(stacks.iter().any(|s| s.role == StackRole::Frontend));
        // 后端排在前端之前。
        assert_eq!(stacks[0].role, StackRole::Backend);

        let sug = suggest_run_config(&d);
        // dev 来自前端，测试来自后端。
        assert_eq!(sug.dev_command.as_deref(), Some("npm run dev"));
        assert_eq!(sug.test_unit.as_deref(), Some("go test ./..."));
        assert_eq!(sug.dev_kind.as_deref(), Some("web"));
        assert!(sug.summary.contains("go") && sug.summary.contains("node-frontend"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn tauri_detected_with_app_command() {
        let d = tmp("tauri");
        write(&d, "src-tauri/tauri.conf.json", "{}");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"dev":"vite","tauri:dev":"tauri dev"}}"#,
        );
        let stacks = detect_stacks(&d);
        let t = stacks.iter().find(|s| s.id == "tauri").unwrap();
        assert_eq!(t.role, StackRole::Desktop);
        assert_eq!(t.dev_command.as_deref(), Some("npm run dev"));
        assert_eq!(t.app_command.as_deref(), Some("npm run tauri:dev"));

        let sug = suggest_run_config(&d);
        assert_eq!(sug.dev_kind.as_deref(), Some("tauri"));
        assert_eq!(sug.app_command.as_deref(), Some("npm run tauri:dev"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn static_site_fallback() {
        let d = tmp("static");
        write(&d, "index.html", "<html></html>");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.role, StackRole::Static);
        assert!(s.dev_command.as_ref().unwrap().contains("http.server"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn dep_cache_only_when_present() {
        let d = tmp("depcache");
        write(&d, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"vue":"^3"}}"#);
        // node_modules 不存在 → 不软链。
        assert!(dep_cache_dirs(&d).is_empty());
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        assert_eq!(dep_cache_dirs(&d), vec!["node_modules".to_string()]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn unknown_project_yields_empty_suggestion() {
        let d = tmp("empty");
        let sug = suggest_run_config(&d);
        assert!(sug.dev_command.is_none());
        assert!(sug.summary.contains("未识别"));
        std::fs::remove_dir_all(&d).ok();
    }
}
