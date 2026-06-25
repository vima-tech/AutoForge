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
    /// 微信小程序（原生 / Taro / uni-app / mpx）。预览语义是「编译产物」而非「可访问 URL」。
    MiniApp,
}

impl StackRole {
    fn as_str(self) -> &'static str {
        match self {
            StackRole::Frontend => "frontend",
            StackRole::Backend => "backend",
            StackRole::Static => "static",
            StackRole::Desktop => "desktop",
            StackRole::MiniApp => "miniapp",
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

/// 检测微信小程序工程（原生 / Taro / uni-app / mpx）。
///
/// 必须排在 `detect_node` 之前短路，否则 Taro/uni-app（都是 Node 工程，有 package.json）
/// 会被当成普通前端。预览语义是「编译产物」而非「可访问 URL」（见 cr_preview 的 miniapp 分支）。
fn detect_wechat_miniapp(dir: &Path) -> Option<DetectedStack> {
    let pkg = read_pkg(dir);
    let deps: Vec<String> = pkg.as_ref().map(|(_, d)| d.clone()).unwrap_or_default();
    let has = |needle: &str| deps.iter().any(|d| d == needle);

    // 框架判定（互斥，按优先级）。
    let (id, framework) = if has("@tarojs/taro") || has("@tarojs/cli") {
        ("wechat-taro", "taro")
    } else if has("@dcloudio/uni-app")
        || (exists(dir, "manifest.json") && exists(dir, "pages.json"))
    {
        ("wechat-uniapp", "uni-app")
    } else if has("@mpxjs/core") {
        ("wechat-mpx", "mpx")
    } else if exists(dir, "project.config.json") && exists(dir, "app.json") {
        // 原生小程序：无构建框架，仅靠开发者工具编译。
        ("wechat-native", "native")
    } else {
        return None;
    };

    let mut s = DetectedStack::base(id, StackRole::MiniApp, "typescript");
    s.framework = framework.to_string();

    if framework == "native" {
        // 原生小程序无 npm scripts；命令留空，预览走开发者工具编译（见 cr_preview）。
        s.language = "javascript".to_string();
        return Some(s);
    }

    // Taro / uni-app / mpx：复用 Node 工程的包管理器与 scripts 解析。
    let pm = package_manager(dir);
    let (scripts, _) = pkg.unwrap_or_default();
    // 小程序专用脚本优先（按框架惯例），回退通用 build/dev。
    let build = first_script(
        &scripts,
        &["build:weapp", "build:mp-weixin", "build:weixin", "build:mp", "build"],
    )
    .map(|sc| pm_run(pm, &sc));
    let dev = first_script(
        &scripts,
        &["dev:weapp", "dev:mp-weixin", "dev:weixin", "dev:mp", "dev"],
    )
    .map(|sc| pm_run(pm, &sc));
    let test = first_script(&scripts, &["test", "test:unit", "vitest", "jest"]).map(|sc| pm_run(pm, &sc));
    let lint = first_script(&scripts, &["lint"]).map(|sc| pm_run(pm, &sc));
    let typing = first_script(&scripts, &["typecheck", "type-check", "tsc"]).map(|sc| pm_run(pm, &sc));

    // dev_command 对小程序仍记录（开发者工具可外接监听），但预览闸口走 build。
    s.dev_command = dev;
    s.build_command = build;
    s.test_unit = test;
    s.lint = lint;
    s.typing = typing;
    s.security = if pm == "npm" { Some("npm audit".to_string()) } else { None };
    s.dep_cache_dirs = vec!["node_modules".to_string()];
    s.scanners = vec!["npm_audit"];
    s.analyzers = vec!["eslint"];
    Some(s)
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
    let is_quarkus = build_text.contains("quarkus");
    let is_micronaut = build_text.contains("micronaut");

    if is_maven {
        let mut s = DetectedStack::base("java-maven", StackRole::Backend, "java");
        if is_spring {
            s.framework = "spring-boot".to_string();
            s.dev_command = Some(
                "mvn -q spring-boot:run -Dspring-boot.run.arguments=--server.port={port}".to_string(),
            );
        } else if is_quarkus {
            s.framework = "quarkus".to_string();
            s.dev_command = Some("mvn -q quarkus:dev -Dquarkus.http.port={port}".to_string());
        } else if is_micronaut {
            s.framework = "micronaut".to_string();
            s.dev_command = Some("mvn -q mn:run".to_string());
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
    } else if is_quarkus {
        s.framework = "quarkus".to_string();
        s.dev_command = Some(format!("{gw} quarkusDev"));
    } else if is_micronaut {
        s.framework = "micronaut".to_string();
        s.dev_command = Some(format!("{gw} run"));
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
    // vendor/ 存在时软链进 worktree，避免分支预览/编译时缺 vendored 依赖。
    // （dep_cache_dirs() 会再校验目录确实存在才纳入。）
    if exists(dir, "vendor") {
        s.dep_cache_dirs = vec!["vendor".to_string()];
    }
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
    } else if dep_lc.contains("starlette") {
        "starlette"
    } else if dep_lc.contains("sanic") {
        "sanic"
    } else if dep_lc.contains("flask") {
        "flask"
    } else {
        ""
    };
    // 命令前缀：uv（uv.lock 或 pyproject 声明 uv）优先于 poetry，否则裸命令。
    let is_uv = exists(dir, "uv.lock")
        || (is_poetry && {
            let head = read_head(dir, "pyproject.toml", 4000).unwrap_or_default();
            head.contains("[tool.uv]") || head.contains("requires = [\"uv")
        });
    let run = if is_uv {
        "uv run "
    } else if is_poetry {
        "poetry run "
    } else {
        ""
    };

    let id = if is_uv {
        "python-uv"
    } else if is_poetry {
        "python-poetry"
    } else {
        "python-pip"
    };
    let mut s = DetectedStack::base(id, StackRole::Backend, "python");
    s.framework = framework.to_string();
    s.dev_command = match framework {
        "django" => Some(format!("{run}python manage.py runserver 0.0.0.0:{{port}}")),
        "fastapi" | "starlette" => Some(format!("{run}uvicorn main:app --reload --port {{port}}")),
        "sanic" => Some(format!("{run}sanic server.app --host 0.0.0.0 --port {{port}}")),
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

    // 微信小程序 > Tauri > 普通前端，三者互斥取一（都基于 package.json，小程序须先短路）。
    if let Some(m) = detect_wechat_miniapp(dir) {
        out.push(m);
    } else if let Some(t) = detect_tauri(dir) {
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

    // 排序：MiniApp/Desktop > Backend > Frontend > Static，保证 primary 取到合理代表。
    // MiniApp 与 Desktop 同为「主交付物」，并列最高优先级。
    fn rank(r: StackRole) -> u8 {
        match r {
            StackRole::MiniApp => 0,
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

/// 把仓库的依赖缓存目录（gitignore 的 node_modules / target 等不在 worktree 内）软链进
/// 目标目录，幂等：已存在则跳过，`dest == repo` 时自然跳过（src==dst 已存在）。
/// 让编码 Agent 与合并前测试门能找到本地 tsc/eslint，否则 `npx tsc` 因缺 node_modules
/// 退化为联网抓占位假包而失败。非 unix 平台为 no-op（symlink 语义不一致，预览同样仅 unix）。
pub fn link_dep_caches(repo: &Path, dest: &Path) {
    #[cfg(unix)]
    {
        // Callers do not always use the same lexical form for the main checkout
        // (for example `/repo` vs `/repo/.`).  Comparing only `Path` values can
        // therefore create an absolute symlink from a cache directory back to
        // itself.  Besides destroying the cache, `git add -A` can then commit
        // that symlink because `dir/` ignore rules do not match a symlink.
        let same_dir = repo == dest
            || match (repo.canonicalize(), dest.canonicalize()) {
                (Ok(repo), Ok(dest)) => repo == dest,
                _ => false,
            };
        if same_dir {
            return;
        }
        for rel in dep_cache_dirs(repo) {
            let src = repo.join(&rel);
            let dst = dest.join(&rel);
            if src.exists() && !dst.exists() {
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::os::unix::fs::symlink(&src, &dst);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (repo, dest);
    }
}

/// Arguments for staging a worktree while explicitly excluding dependency-cache
/// links.  This is a second safety boundary in addition to `.gitignore`: older
/// CR branches may predate the fixed ignore rules, and generated cache symlinks
/// must never become product changes.
pub fn git_add_all_args(dir: &Path) -> Vec<String> {
    let mut args = vec![
        "add".to_string(),
        "-A".to_string(),
        "--".to_string(),
        ".".to_string(),
    ];
    args.extend(
        dep_cache_dirs(dir)
            .into_iter()
            .map(|rel| format!(":(exclude){rel}")),
    );
    args
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

/// 生成一句人类可读的**应用品类**描述（供运行配置只读展示，区别于"预览方式"）。
/// 基于 `detect_stacks` 的角色/语言/框架，把 web 这种过宽的预览类目细分为可理解的品类：
/// 前端 / 后台前端无法靠依赖可靠区分，故统一"前端"；后端有 web 框架→"后端服务"，
/// 无框架（裸语言工程）→"后端/脚本/库"（诚实：CLI、脚本、库无法可靠区分）。
pub fn detected_category(dir: &Path) -> String {
    let stacks = detect_stacks(dir);
    let Some(primary) = stacks.first() else {
        return "未识别".to_string();
    };
    let fe = stacks.iter().find(|s| s.role == StackRole::Frontend);
    let be = stacks.iter().find(|s| s.role == StackRole::Backend);
    let fw_or = |s: &DetectedStack, fallback: &str| {
        if s.framework.is_empty() {
            fallback.to_string()
        } else {
            s.framework.clone()
        }
    };

    match primary.role {
        StackRole::MiniApp => format!("微信小程序 · {}", fw_or(primary, "原生")),
        StackRole::Desktop => "桌面应用 · Tauri".to_string(),
        StackRole::Static => "静态站".to_string(),
        _ => match (fe, be) {
            // 同时含前后端 = 全栈。
            (Some(f), Some(b)) => format!(
                "全栈（前端+后端） · {} + {}",
                fw_or(f, &f.language),
                fw_or(b, &b.language)
            ),
            // 仅后端：有 web 框架 = 服务；裸语言工程无法区分服务/脚本/库。
            (None, Some(b)) => {
                if b.framework.is_empty() {
                    format!("后端 / 脚本 / 库 · {}", b.language)
                } else {
                    format!("后端服务 · {}/{}", b.language, b.framework)
                }
            }
            // 仅前端（含后台前端，技术栈上不可靠区分，统一"前端"）。
            (Some(f), None) => format!("前端 · {}", fw_or(f, &f.language)),
            // 兜底：用 primary。
            _ => format!("{} · {}", primary.language, fw_or(primary, &primary.id)),
        },
    }
}

/// 生成「技术栈画像 + 默认编码约定」段，供**分析阶段**（build_project_context）与
/// **执行阶段**（build_prompt）注入同一段，保证两阶段对栈的认知一致。
///
/// 设计：
/// - 只陈述**可验证事实**（检测到哪些栈/框架），不臆断领域（如"网站 vs 后台"无法靠依赖区分）。
/// - 每栈默认约定 ≤ 数行，防 prompt 膨胀；明确声明**项目 CLAUDE.md / .autoforge/specs 冲突时优先**。
/// - 未识别到栈时返回空串（调用方据此不输出标题）。
pub fn stack_hint(dir: &Path) -> String {
    let stacks = detect_stacks(dir);
    if stacks.is_empty() {
        return String::new();
    }
    use std::fmt::Write;
    let mut s = String::new();
    s.push_str("检测到以下技术栈（自动嗅探，仅供参考）：\n");
    for st in &stacks {
        let fw = if st.framework.is_empty() {
            String::new()
        } else {
            format!(" / {}", st.framework)
        };
        let _ = writeln!(s, "- {} · {}{} （{}）", st.id, st.language, fw, st.role.as_str());
    }

    // 按检测到的栈挂默认约定（去重，每条约定只出一次）。
    let mut hinted: Vec<&str> = Vec::new();
    for st in &stacks {
        let key = stack_hint_key(st);
        if key.is_empty() || hinted.contains(&key) {
            continue;
        }
        if let Some(block) = default_conventions(key) {
            hinted.push(key);
            s.push('\n');
            s.push_str(block);
            s.push('\n');
        }
    }

    s.push_str(
        "\n> 以上为按栈嗅探的**默认约定**，是参考而非铁律；\
         项目 `CLAUDE.md` 与 `.autoforge/specs` 若有不同规定，**一律以后者为准**。\n",
    );
    s
}

/// 把一个检测到的栈映射到默认约定的 key（按 framework 优先，其次 language/role）。
fn stack_hint_key(st: &DetectedStack) -> &'static str {
    match st.role {
        StackRole::MiniApp => "miniapp",
        StackRole::Desktop => "tauri",
        _ => match st.framework.as_str() {
            "django" => "django",
            "fastapi" | "starlette" => "fastapi",
            "flask" => "flask",
            "spring-boot" => "spring-boot",
            "quarkus" => "quarkus",
            "gin" | "echo" | "fiber" => "go-web",
            "vue" => "vue",
            "react" | "next" => "react",
            _ => match st.language.as_str() {
                "go" => "go-web",
                "python" => "python",
                "java" => "java",
                "rust" => "rust",
                _ if st.role == StackRole::Frontend => "frontend",
                _ => "",
            },
        },
    }
}

/// 各 key 对应的精炼默认约定（≤ 数行）。纯字符串常量。
fn default_conventions(key: &str) -> Option<&'static str> {
    let block = match key {
        "miniapp" => "### 微信小程序约定\n\
            - 禁止浏览器 API：不得用 `document` / `window` / `fetch` / `localStorage`；\
            改用 `Taro.*` / `wx.*`（导航 `Taro.navigateTo`、存储 `Taro.setStorage`、请求 `Taro.request`）。\n\
            - 页面结构：原生=`page/{name}.{js,wxml,wxss,json}`；Taro=`pages/{name}/index.tsx` + `index.config.ts`。\n\
            - 样式用 `rpx` 单位；网络统一经 service 层封装 `Taro.request`（注入登录 token）。\n\
            - 改动后必须能通过小程序编译（`build:weapp` / `build:mp-weixin`）。",
        "tauri" => "### Tauri 桌面应用约定\n\
            - 严格使用 Tauri 2.x API（`@tauri-apps/api/core` 的 `invoke`、`@tauri-apps/api/window` 的 `getCurrentWindow`）；禁用 1.x。\n\
            - 新增 JS→Rust 调用须在 `src-tauri/capabilities/*.json` 声明权限，否则运行时 not allowed。",
        "react" => "### React 前端约定\n\
            - 函数组件 + Hooks；副作用放 `useEffect` 并清理；列表渲染带稳定 `key`。\n\
            - API 调用收敛到 service 层，不在组件内散落 fetch。",
        "vue" => "### Vue 前端约定\n\
            - Vue 3 Composition API（`<script setup>`）；状态用 Pinia；避免直接操作 DOM。\n\
            - API 调用收敛到 service/composable 层。",
        "frontend" => "### 前端约定\n\
            - 组件化、状态与视图分离；API 调用走统一 service 层而非散落组件内。",
        "fastapi" => "### FastAPI 约定\n\
            - 路由用 async def；I/O 用 await 不阻塞事件循环；请求/响应用 Pydantic 模型。\n\
            - 依赖用 `Depends` 注入；按 router 模块拆分。",
        "django" => "### Django 约定\n\
            - 遵循 app 结构；模型变更必须生成并提交 migration（`manage.py makemigrations`）。\n\
            - 业务逻辑放 service/manager，视图保持瘦。",
        "flask" => "### Flask 约定\n\
            - 用 Blueprint 组织路由；扩展用 app factory 初始化；配置与代码分离。",
        "spring-boot" => "### Spring Boot 约定\n\
            - 分层 Controller/Service/Repository；构造器注入而非字段注入。\n\
            - DTO 与实体分离；统一异常处理。",
        "quarkus" => "### Quarkus 约定\n\
            - 优先 CDI 注入与 JAX-RS 资源；遵循 Quarkus 扩展惯例，避免重型反射。",
        "go-web" => "### Go Web 约定\n\
            - 错误显式返回并包装（`fmt.Errorf(\"...: %w\", err)`），不吞错；\
            handler 瘦、逻辑下沉到 service。\n\
            - 端口走 `PORT` 环境变量；并发注意 context 取消与 goroutine 泄漏。",
        "python" => "### Python 约定\n\
            - 类型注解 + 遵循既有依赖管理（poetry/uv/pip）；I/O 密集优先 async。",
        "java" => "### Java 约定\n\
            - 遵循既有构建工具（Maven/Gradle）；分层清晰、依赖注入而非 new。",
        "rust" => "### Rust 约定\n\
            - 错误用 `Result` + `?` 传播，避免 `unwrap()` 在非测试路径；遵循 `cargo clippy`。",
        _ => return None,
    };
    Some(block)
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
    let ui = stacks.iter().find(|s| {
        matches!(
            s.role,
            StackRole::Frontend | StackRole::Desktop | StackRole::Static | StackRole::MiniApp
        )
    });
    let backend = stacks.iter().find(|s| s.role == StackRole::Backend);

    let dev_kind = match ui.map(|s| s.role) {
        Some(StackRole::Desktop) => Some("tauri".to_string()),
        // 小程序预览语义是「编译产物」而非「可访问 URL」，独立 kind（cr_preview 走一次性 build）。
        Some(StackRole::MiniApp) => Some("miniapp".to_string()),
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
    fn git_add_excludes_detected_dependency_caches() {
        let d = tmp("git-add-cache-excludes");
        write(&d, "src-tauri/tauri.conf.json", "{}");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"dev":"vite","tauri:dev":"tauri dev"}}"#,
        );
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        std::fs::create_dir_all(d.join("src-tauri/target")).unwrap();

        let args = git_add_all_args(&d);
        assert_eq!(&args[..4], ["add", "-A", "--", "."]);
        assert!(args.contains(&":(exclude)node_modules".to_string()));
        assert!(args.contains(&":(exclude)src-tauri/target".to_string()));

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_wechat_taro() {
        let d = tmp("taro");
        write(&d, "project.config.json", r#"{"miniprogramRoot":"dist"}"#);
        write(
            &d,
            "package.json",
            r#"{"scripts":{"build:weapp":"taro build --type weapp","dev:weapp":"taro build --type weapp --watch"},"dependencies":{"@tarojs/taro":"^4"}}"#,
        );
        let stacks = detect_stacks(&d);
        let s = &stacks[0];
        assert_eq!(s.id, "wechat-taro");
        assert_eq!(s.role, StackRole::MiniApp);
        assert_eq!(s.framework, "taro");
        assert_eq!(s.build_command.as_deref(), Some("npm run build:weapp"));
        assert_eq!(s.dep_cache_dirs, vec!["node_modules".to_string()]);
        let sug = suggest_run_config(&d);
        assert_eq!(sug.dev_kind.as_deref(), Some("miniapp"));
        assert_eq!(sug.build_command.as_deref(), Some("npm run build:weapp"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_wechat_uniapp() {
        let d = tmp("uniapp");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"build:mp-weixin":"uni build -p mp-weixin"},"dependencies":{"@dcloudio/uni-app":"^3"}}"#,
        );
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.id, "wechat-uniapp");
        assert_eq!(s.role, StackRole::MiniApp);
        assert_eq!(s.build_command.as_deref(), Some("npm run build:mp-weixin"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_wechat_native() {
        let d = tmp("wxnative");
        write(&d, "project.config.json", r#"{"appid":"wx123"}"#);
        write(&d, "app.json", r#"{"pages":["pages/index/index"]}"#);
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.id, "wechat-native");
        assert_eq!(s.role, StackRole::MiniApp);
        assert_eq!(s.framework, "native");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn taro_not_stolen_by_node_detection() {
        // Taro 工程有 react 依赖也不能被当成普通前端。
        let d = tmp("taro-react");
        write(&d, "project.config.json", "{}");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"build:weapp":"taro build"},"dependencies":{"@tarojs/taro":"^4","react":"^18"}}"#,
        );
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.role, StackRole::MiniApp);
        assert_eq!(s.id, "wechat-taro");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_python_uv() {
        let d = tmp("pyuv");
        write(&d, "uv.lock", "version = 1\n");
        write(&d, "pyproject.toml", "[project]\nname='x'\ndependencies=['fastapi']\n");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.id, "python-uv");
        assert_eq!(s.framework, "fastapi");
        assert!(s.dev_command.as_ref().unwrap().starts_with("uv run "));
        assert_eq!(s.test_unit.as_deref(), Some("uv run pytest"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detects_java_quarkus_maven() {
        let d = tmp("quarkus");
        write(&d, "pom.xml", "<project><dependency>io.quarkus:quarkus-resteasy</dependency></project>");
        let s = &detect_stacks(&d)[0];
        assert_eq!(s.framework, "quarkus");
        assert!(s.dev_command.as_ref().unwrap().contains("quarkus:dev"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn go_vendor_added_to_dep_cache() {
        let d = tmp("govendor");
        write(&d, "go.mod", "module x\nrequire github.com/gin-gonic/gin v1\n");
        std::fs::create_dir_all(d.join("vendor")).unwrap();
        assert_eq!(dep_cache_dirs(&d), vec!["vendor".to_string()]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn stack_hint_includes_profile_and_defers_to_project() {
        let d = tmp("hint");
        write(&d, "project.config.json", "{}");
        write(
            &d,
            "package.json",
            r#"{"scripts":{"build:weapp":"taro build"},"dependencies":{"@tarojs/taro":"^4"}}"#,
        );
        let hint = stack_hint(&d);
        assert!(hint.contains("wechat-taro"));
        assert!(hint.contains("微信小程序约定"));
        assert!(hint.contains("以后者为准")); // 冲突让位声明
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn stack_hint_empty_for_unknown() {
        let d = tmp("hint-empty");
        assert!(stack_hint(&d).is_empty());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detected_category_distinguishes_kinds() {
        // 后端服务（有 web 框架）。
        let d = tmp("cat-be");
        write(&d, "go.mod", "module x\nrequire github.com/gin-gonic/gin v1\n");
        assert_eq!(detected_category(&d), "后端服务 · go/gin");
        std::fs::remove_dir_all(&d).ok();

        // 裸语言工程（无框架）→ 后端/脚本/库。
        let d = tmp("cat-script");
        write(&d, "go.mod", "module x\n");
        assert_eq!(detected_category(&d), "后端 / 脚本 / 库 · go");
        std::fs::remove_dir_all(&d).ok();

        // 全栈（前端 + 后端）。
        let d = tmp("cat-full");
        write(&d, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"react":"^18"}}"#);
        write(&d, "go.mod", "module x\nrequire github.com/gin-gonic/gin v1\n");
        assert!(detected_category(&d).starts_with("全栈"));
        std::fs::remove_dir_all(&d).ok();

        // 微信小程序。
        let d = tmp("cat-mini");
        write(&d, "project.config.json", "{}");
        write(&d, "package.json", r#"{"dependencies":{"@tarojs/taro":"^4"}}"#);
        assert_eq!(detected_category(&d), "微信小程序 · taro");
        std::fs::remove_dir_all(&d).ok();

        // 前端。
        let d = tmp("cat-fe");
        write(&d, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"vue":"^3"}}"#);
        assert_eq!(detected_category(&d), "前端 · vue");
        std::fs::remove_dir_all(&d).ok();

        // 未识别。
        let d = tmp("cat-none");
        assert_eq!(detected_category(&d), "未识别");
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
