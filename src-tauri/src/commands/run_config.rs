use crate::models::project::Project;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

/// 运行配置生成复用「部署官」(deploy) 系统角色的 LLM 绑定。
const RUN_CONFIG_ROLE: &str = "deploy";

// ── 运行配置持久化（真源 = 仓库内 .autoforge/run-config.json）───────────────────
//
// 文件存 JSON；JSON 是合法 YAML，故所有 `serde_yaml::from_str` 消费者可直接解析。
// 读取以文件为准、缺失时回退旧 DB 列（未迁移项目）；保存时写文件并把 DB 列作镜像。

fn config_file_path(repo_path: &str) -> Option<PathBuf> {
    let repo = repo_path.trim();
    if repo.is_empty() {
        return None;
    }
    Some(Path::new(repo).join(".autoforge").join("run-config.json"))
}

/// 读取 `.autoforge/run-config.json` 原始内容（JSON 文本）。不存在/空 → None。
pub fn read_config_file(repo_path: &str) -> Option<String> {
    let path = config_file_path(repo_path)?;
    let text = std::fs::read_to_string(&path).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 项目的有效运行配置字符串：文件为真源，缺失时回退到旧的 DB 列。
/// 返回值可能是 JSON（新）或 YAML（旧 DB 列）——两者皆为合法 YAML，消费者统一用 serde_yaml 解析。
pub fn effective_config(project: &Project) -> Option<String> {
    read_config_file(&project.repo_path).or_else(|| {
        project
            .config_yaml
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
    })
}

/// 持久化运行配置到 `.autoforge/run-config.json`（统一规整为美化 JSON）。
/// 空配置删除文件；仓库路径无效时静默跳过（仅靠 DB 列兜底）。
pub fn write_config_file(repo_path: &str, content: &str) -> Result<(), String> {
    let Some(path) = config_file_path(repo_path) else {
        return Ok(());
    };
    if content.trim().is_empty() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        return Ok(());
    }
    // content 可能是 JSON 或旧 YAML —— serde_yaml 都能解析，统一输出美化 JSON。
    let value: serde_json::Value =
        serde_yaml::from_str(content).map_err(|e| format!("运行配置解析失败: {e}"))?;
    let json =
        serde_json::to_string_pretty(&value).map_err(|e| format!("运行配置序列化失败: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建 .autoforge 目录失败: {e}"))?;
    }
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("写入 run-config.json 失败: {e}"))?;
    Ok(())
}

/// AI 推断出的运行配置草稿。全部可选——拿不准的字段留空，由人工补全。
/// 字段名对齐前端 ProjectConfigForm（脱敏字段不交给 AI 猜，由人工填写）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RunConfigDraft {
    #[serde(default)]
    pub dev_kind: Option<String>,
    #[serde(default)]
    pub dev_command: Option<String>,
    #[serde(default)]
    pub app_command: Option<String>,
    #[serde(default)]
    pub test_unit: Option<String>,
    #[serde(default)]
    pub test_unit_timeout: Option<u64>,
    #[serde(default)]
    pub test_integration: Option<String>,
    #[serde(default)]
    pub test_integration_timeout: Option<u64>,
    #[serde(default)]
    pub quality_lint: Option<String>,
    #[serde(default)]
    pub quality_typing: Option<String>,
    #[serde(default)]
    pub quality_security: Option<String>,
    #[serde(default)]
    pub project_language: Option<String>,
    #[serde(default)]
    pub project_framework: Option<String>,
    #[serde(default)]
    pub preview_build: Option<String>,
    #[serde(default)]
    pub preview_start: Option<String>,
    #[serde(default)]
    pub deploy_command: Option<String>,
}

/// 根据项目仓库的构建文件，让 AI 推断一份「运行配置」草稿返回给前端填表（不落库，由人工审阅后保存）。
#[tauri::command]
pub async fn ai_generate_run_config(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<RunConfigDraft, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("项目不存在")?;

    // 栈画像检测：先用 core::stack 给出一份**可靠的默认运行配置建议**，
    // 既作为草稿底座（AI 留空也有合理默认），又作为 AI 的判断依据，避免空猜。
    let suggestion = if project.repo_path.is_empty() {
        crate::core::stack::RunConfigSuggestion::default()
    } else {
        crate::core::stack::suggest_run_config(std::path::Path::new(&project.repo_path))
    };

    // 扫描仓库关键构建文件，给 AI 可靠依据（每个截前 1200 字）。
    let mut hints: Vec<String> = Vec::new();
    let mut is_tauri = false;
    if !project.repo_path.is_empty() {
        let repo = std::path::Path::new(&project.repo_path);
        if repo.join("src-tauri/tauri.conf.json").exists()
            || repo.join("src-tauri/Cargo.toml").exists()
        {
            is_tauri = true;
        }
        for name in &[
            "package.json",
            "Cargo.toml",
            "pyproject.toml",
            "requirements.txt",
            "go.mod",
            "pom.xml",
            "Makefile",
        ] {
            if let Ok(text) = std::fs::read_to_string(repo.join(name)) {
                let truncated: String = text.chars().take(1200).collect();
                hints.push(format!("=== {} ===\n{}", name, truncated));
            }
        }
    }
    let hints_text = if hints.is_empty() {
        "无（仓库未识别到构建文件，请尽量从项目描述推断）".to_string()
    } else {
        hints.join("\n\n")
    };

    // 把栈画像建议序列化进提示：AI 在此基础上确认/修正，而不是从零猜。
    let suggested_json = serde_json::json!({
        "dev_kind": suggestion.dev_kind,
        "dev_command": suggestion.dev_command,
        "app_command": suggestion.app_command,
        "test_unit": suggestion.test_unit,
        "test_integration": suggestion.test_integration,
        "quality_lint": suggestion.lint,
        "quality_typing": suggestion.typing,
        "quality_security": suggestion.security,
        "project_language": suggestion.language,
        "project_framework": suggestion.framework,
        "preview_build": suggestion.build_command,
        "preview_start": suggestion.start_command,
    });

    let prompt = format!(
        r#"根据项目信息与仓库构建文件，推断 AutoForge「运行配置」并输出 JSON。

项目名称：{name}
项目描述：{desc}
检测到 Tauri 桌面项目：{tauri}

自动栈检测：{stack_summary}
基于栈检测的默认建议（合理则沿用，不合理则按仓库实际修正）：
{suggested}

仓库构建文件摘要：
{hints}

字段说明（无法确定的字段置 null，不要编造命令）：
- dev_kind: "web" 或 "tauri"
- dev_command: 开发/前端预览启动命令；端口位置用 {{port}} 占位，如 "npm run dev -- --port {{port}}"
- app_command: 仅 tauri，桌面应用启动命令，如 "npm run tauri:dev"
- test_unit / test_integration: 单元 / 集成测试命令
- test_unit_timeout / test_integration_timeout: 超时秒数（整数）
- quality_lint / quality_typing / quality_security: lint / 类型检查 / 安全扫描命令
- project_language / project_framework: 语言与框架，如 "typescript" / "react"
- preview_build / preview_start: 部署用的构建命令与启动命令
- deploy_command: 完整自定义部署命令（存在则优先于 preview_*）

只输出一个 JSON 对象，不要解释、不要 markdown 代码块：
{{"dev_kind":"web","dev_command":null,"app_command":null,"test_unit":null,"test_unit_timeout":null,"test_integration":null,"test_integration_timeout":null,"quality_lint":null,"quality_typing":null,"quality_security":null,"project_language":null,"project_framework":null,"preview_build":null,"preview_start":null,"deploy_command":null}}"#,
        name = project.name,
        desc = project.description,
        tauri = is_tauri,
        stack_summary = suggestion.summary,
        suggested = serde_json::to_string_pretty(&suggested_json).unwrap_or_default(),
        hints = hints_text,
    );

    let raw = crate::agents::llm::run_system_role_text(
        &state.db,
        RUN_CONFIG_ROLE,
        &prompt,
        Some("你是 AutoForge 的运行配置助手，依据项目仓库文件推断构建/测试/预览/部署命令。只输出调用方要求的 JSON。"),
        Some(&project_id),
        Some(&format!("运行配置 构建 测试 部署 {}", project.name)),
    )
    .await
    .map_err(|e| format!("AI 生成失败: {}", e))?;

    let start = raw.find('{').ok_or("AI 返回格式错误，未找到 JSON")?;
    let end = raw.rfind('}').ok_or("AI 返回 JSON 不完整")?;
    let ai: RunConfigDraft =
        serde_json::from_str(&raw[start..=end]).map_err(|e| format!("解析 AI 输出失败: {}", e))?;

    // 合并：以栈检测建议为底座，AI 给出的非空字段覆盖之。这样即便 AI 漏填，
    // 检测出的可靠默认（如 `mvn -q test` / `go test ./...`）仍会保留。
    let draft = merge_draft(&suggestion, ai);
    Ok(draft)
}

/// 用栈检测建议作底座，AI 非空字段覆盖之。脱敏/超时类字段只由 AI 给（建议里没有）。
fn merge_draft(s: &crate::core::stack::RunConfigSuggestion, ai: RunConfigDraft) -> RunConfigDraft {
    fn pick(ai: Option<String>, base: Option<String>) -> Option<String> {
        ai.filter(|v| !v.trim().is_empty()).or(base)
    }
    RunConfigDraft {
        dev_kind: pick(ai.dev_kind, s.dev_kind.clone()),
        dev_command: pick(ai.dev_command, s.dev_command.clone()),
        app_command: pick(ai.app_command, s.app_command.clone()),
        test_unit: pick(ai.test_unit, s.test_unit.clone()),
        test_unit_timeout: ai.test_unit_timeout,
        test_integration: pick(ai.test_integration, s.test_integration.clone()),
        test_integration_timeout: ai.test_integration_timeout,
        quality_lint: pick(ai.quality_lint, s.lint.clone()),
        quality_typing: pick(ai.quality_typing, s.typing.clone()),
        quality_security: pick(ai.quality_security, s.security.clone()),
        project_language: pick(ai.project_language, s.language.clone()),
        project_framework: pick(ai.project_framework, s.framework.clone()),
        preview_build: pick(ai.preview_build, s.build_command.clone()),
        preview_start: pick(ai.preview_start, s.start_command.clone()),
        deploy_command: ai.deploy_command,
    }
}
