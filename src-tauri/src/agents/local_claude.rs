use anyhow::Result;
use tokio::process::Command;

/// Run claude CLI in text-only mode, returns stdout
pub async fn run_text(prompt: &str, system_prompt: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("claude");
    cmd.arg("--print")
        .arg("--permission-mode")
        .arg("dontAsk")
        .arg("--tools")
        .arg("")
        .arg("--disable-slash-commands")
        .arg("--no-session-persistence");

    if let Some(sp) = system_prompt {
        cmd.arg("--system-prompt").arg(sp);
    }

    cmd.arg(prompt);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("claude CLI failed: {}", stderr));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

/// Check if claude CLI is authenticated
pub async fn check_auth() -> bool {
    let output = Command::new("claude").arg("--version").output().await;
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}
