use crate::models::deployment::Deployment;
use crate::models::project::Project;
use crate::state::AppState;
use serde::Deserialize;
use std::time::Duration;
use tauri::State;
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
struct DeployYaml {
    project: Option<ProjectSection>,
    preview: Option<PreviewSection>,
    deploy: Option<DeploySection>,
}
#[derive(Debug, Deserialize, Default)]
struct ProjectSection {
    language: Option<String>,
    framework: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
struct PreviewSection {
    build: Option<CmdSection>,
    start: Option<CmdSection>,
}
#[derive(Debug, Deserialize, Default)]
struct DeploySection {
    command: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
struct CmdSection {
    command: Option<String>,
}

#[tauri::command]
pub async fn list_deployments(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Deployment>, String> {
    match project_id {
        Some(pid) => sqlx::query_as::<_, Deployment>(
            "SELECT * FROM deployments WHERE project_id=? ORDER BY created_at DESC LIMIT 200",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        None => sqlx::query_as::<_, Deployment>(
            "SELECT * FROM deployments ORDER BY created_at DESC LIMIT 200",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}

/// Generate (but do NOT run) a deploy shell script. Stored as `pending_confirm`.
/// Execution requires an explicit `confirm_deploy` — the third human gate (design §4.3 Layer 3).
#[tauri::command]
pub async fn generate_deploy_script(
    project_id: String,
    target_env: Option<String>,
    state: State<'_, AppState>,
) -> Result<Deployment, String> {
    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project_id))?;

    let env = target_env.unwrap_or_else(|| "production".to_string());
    let cfg: DeployYaml = project
        .config_yaml
        .as_deref()
        .and_then(|y| serde_yaml::from_str(y).ok())
        .unwrap_or_default();

    let heuristic = heuristic_script(&project, &cfg, &env);
    // Best-effort LLM polish — falls back to the heuristic when no deploy agent is configured.
    let script = llm_script(&state.db, &project, &cfg, &env, &heuristic)
        .await
        .unwrap_or(heuristic);

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO deployments (id, project_id, target_env, script, status)
         VALUES (?, ?, ?, ?, 'pending_confirm')",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(&env)
    .bind(&script)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Deployment>("SELECT * FROM deployments WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Execute a previously-generated deploy script. This is the gated, irreversible action.
#[tauri::command]
pub async fn confirm_deploy(
    deployment_id: String,
    state: State<'_, AppState>,
) -> Result<Deployment, String> {
    let dep = sqlx::query_as::<_, Deployment>("SELECT * FROM deployments WHERE id=?")
        .bind(&deployment_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("deployment {} not found", deployment_id))?;

    if dep.status != "pending_confirm" {
        return Err(format!("部署 {} 状态为 {}，不可执行", dep.id, dep.status));
    }

    let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id=?")
        .bind(&dep.project_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE deployments SET status='running', confirmed_at=datetime('now') WHERE id=?",
    )
    .bind(&deployment_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let output = tokio::time::timeout(
        Duration::from_secs(600),
        Command::new("sh")
            .arg("-lc")
            .arg(&dep.script)
            .current_dir(&project.repo_path)
            .output(),
    )
    .await;

    let (status, log) = match output {
        Ok(Ok(out)) => {
            let mut log = String::new();
            log.push_str(&String::from_utf8_lossy(&out.stdout));
            if !out.stderr.is_empty() {
                log.push_str("\n[stderr]\n");
                log.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            let status = if out.status.success() {
                "deployed"
            } else {
                "failed"
            };
            (status, log.chars().take(20000).collect::<String>())
        }
        Ok(Err(e)) => ("failed", format!("执行失败: {e}")),
        Err(_) => ("failed", "部署超时（600s）".to_string()),
    };

    sqlx::query(
        "UPDATE deployments SET status=?, log=?, completed_at=datetime('now') WHERE id=?",
    )
    .bind(status)
    .bind(&log)
    .bind(&deployment_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Innate: 部署成功 = 该脚本对本项目目标环境可用，捕获为部署角色的成功样本。
    if status == "deployed" {
        let content = format!(
            "在 {} 环境成功执行的部署脚本：\n\n```bash\n{}\n```",
            dep.target_env, dep.script
        );
        let trigger = format!("为该项目生成 {} 环境部署脚本时可参考", dep.target_env);
        crate::knowledge::kb_add(&dep.project_id, &content, &trigger).await;
    }

    sqlx::query_as::<_, Deployment>("SELECT * FROM deployments WHERE id=?")
        .bind(&deployment_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

fn heuristic_script(project: &Project, cfg: &DeployYaml, env: &str) -> String {
    let build = cfg
        .preview
        .as_ref()
        .and_then(|p| p.build.as_ref())
        .and_then(|c| c.command.clone());
    let start = cfg
        .preview
        .as_ref()
        .and_then(|p| p.start.as_ref())
        .and_then(|c| c.command.clone());
    let explicit = cfg.deploy.as_ref().and_then(|d| d.command.clone());

    let mut s = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    s.push_str(&format!("# AutoForge deploy — {} ({})\n", project.name, env));
    s.push_str(&format!("git checkout {}\n", project.branch_main));
    s.push_str("git pull --ff-only\n");
    if let Some(explicit) = explicit {
        s.push_str(&explicit);
        s.push('\n');
        return s;
    }
    if let Some(build) = build {
        s.push_str(&build);
        s.push('\n');
    }
    if let Some(start) = start {
        // Background long-running start so the deploy command returns.
        s.push_str(&format!("nohup {} >/tmp/autoforge-{}.log 2>&1 &\n", start, project.slug));
    }
    s.push_str("echo 'AutoForge deploy script finished'\n");
    s
}

async fn llm_script(
    db: &crate::db::Db,
    project: &Project,
    cfg: &DeployYaml,
    env: &str,
    heuristic: &str,
) -> Option<String> {
    let lang = cfg
        .project
        .as_ref()
        .and_then(|p| p.language.clone())
        .unwrap_or_default();
    let framework = cfg
        .project
        .as_ref()
        .and_then(|p| p.framework.clone())
        .unwrap_or_default();
    let prompt = format!(
        "为项目「{}」生成一个稳定、幂等的 bash 部署脚本，部署到 {} 环境。\
        \n语言/框架：{} / {}。主分支：{}。\
        \n要求：set -euo pipefail；先切换并拉取主分支；构建；以后台方式启动长驻服务；失败应非零退出。\
        \n只输出脚本本体（可用 ```bash 包裹），不要解释。\n\n参考脚手架：\n{}",
        project.name, env, lang, framework, project.branch_main, heuristic
    );
    // 聚焦召回键：技术栈 + 环境，命中本项目历史部署经验/坑。
    let recall_q = format!("部署 {} {} {}", lang, framework, env);
    let raw = crate::agents::llm::run_system_role_text(
        db,
        "deploy",
        &prompt,
        Some("你是发布工程师，只输出可直接执行的 bash 部署脚本。"),
        Some(&project.id),
        Some(&recall_q),
    )
    .await
    .ok()?;
    let script = strip_code_fence(&raw);
    if script.trim().is_empty() {
        None
    } else {
        Some(script)
    }
}

fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // drop optional language tag on the first line
        let body = rest.splitn(2, '\n').nth(1).unwrap_or("");
        return body.trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}
