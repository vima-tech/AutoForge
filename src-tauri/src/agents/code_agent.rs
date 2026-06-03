use anyhow::Result;
use std::time::Duration;
use tokio::process::Command;

pub fn build_prompt(
    title: &str,
    desc: &str,
    analysis_summary: &str,
    admin_suggestions: Option<&str>,
    iteration: u32,
    repo_path: &str,
    project_config: Option<&str>,
) -> String {
    let mut prompt = format!(
        r#"# 需求实现任务

## 需求标题
{}

## 需求描述
{}

## 分析摘要
{}
"#,
        title, desc, analysis_summary
    );

    if let Some(s) = admin_suggestions {
        if !s.is_empty() {
            prompt.push_str(&format!("\n## 管理员建议\n{}\n", s));
        }
    }

    if iteration > 1 {
        prompt.push_str(&format!(
            "\n## 注意\n这是第 {} 次迭代，请参考之前的实现继续改进。\n",
            iteration
        ));
    }

    if let Some(spec) = read_factory_spec("coding-spec.md") {
        prompt.push_str(&format!("\n## 编码规范\n{}\n", spec));
    }
    if let Some(spec) = read_factory_spec("testing-spec.md") {
        prompt.push_str(&format!("\n## 测试规范\n{}\n", spec));
    }
    if let Some(context) = read_project_file(repo_path, "CLAUDE.md") {
        prompt.push_str(&format!("\n## 目标项目 CLAUDE.md\n{}\n", context));
    }
    let discovered_config = read_project_file(repo_path, "autoforge.yaml");
    if let Some(config) = project_config.or(discovered_config.as_deref()) {
        prompt.push_str(&format!("\n## autoforge.yaml / 项目配置快照\n{}\n", config));
    }

    prompt.push_str(
        r#"
## 要求
1. 在当前 worktree 中实现上述需求
2. 编写必要的测试
3. 完成后输出实现报告，格式如下：

## 改动摘要
（简述做了什么）

## 修改文件列表
（列出修改的文件）

## 测试情况
（测试结果）

## 潜在风险
（可能的风险点）
"#,
    );

    prompt
}

fn read_factory_spec(name: &str) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let root = if cwd.file_name().and_then(|v| v.to_str()) == Some("src-tauri") {
        cwd.parent()?.to_path_buf()
    } else {
        cwd
    };
    std::fs::read_to_string(root.join("specs").join(name)).ok()
}

fn read_project_file(repo_path: &str, name: &str) -> Option<String> {
    std::fs::read_to_string(std::path::Path::new(repo_path).join(name)).ok()
}

/// Run claude code agent in a worktree
/// Returns (exit_code, stdout, stderr)
pub async fn run(
    worktree_path: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<(i32, String, String)> {
    let mut cmd = Command::new("claude");
    cmd.arg("--print")
        .arg("--permission-mode")
        .arg("acceptEdits")
        .arg("--disallowedTools")
        .arg("Bash(git *)")
        .arg(prompt)
        .current_dir(worktree_path);

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("claude code agent timed out after {}s", timeout_secs))??;

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((code, stdout, stderr))
}

/// Extract the report section starting at "## 改动摘要"
pub fn extract_report(output: &str) -> &str {
    if let Some(pos) = output.find("## 改动摘要") {
        &output[pos..]
    } else {
        output
    }
}
