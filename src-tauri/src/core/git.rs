use anyhow::{anyhow, Result};
use std::collections::HashMap;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitSecurityViolation {
    #[error("forbidden git operation: {0}")]
    Forbidden(String),
}

pub struct GitProxy {
    pub repo_path: String,
}

impl GitProxy {
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    fn check_args(&self, args: &[&str]) -> Result<(), GitSecurityViolation> {
        let joined = args.join(" ");

        // Forbid push to main/master
        if args.first() == Some(&"push") {
            let rest = &args[1..].join(" ");
            if rest.contains("main") || rest.contains("master") {
                return Err(GitSecurityViolation::Forbidden(format!(
                    "push to main/master is not allowed: {}",
                    joined
                )));
            }
            // Forbid force push
            if args.iter().any(|a| *a == "--force" || *a == "-f") {
                return Err(GitSecurityViolation::Forbidden(format!(
                    "force push is not allowed: {}",
                    joined
                )));
            }
        }

        // Forbid branch -D except autoforge/* prefix
        if args.first() == Some(&"branch") {
            if args.iter().any(|a| *a == "-D") {
                let branch_name = args.last().unwrap_or(&"");
                if !branch_name.starts_with("autoforge/") {
                    return Err(GitSecurityViolation::Forbidden(format!(
                        "branch -D only allowed for autoforge/* branches: {}",
                        joined
                    )));
                }
            }
        }

        // Forbid dangerous operations
        let forbidden_subcmds = ["symbolic-ref", "update-ref"];
        if let Some(first) = args.first() {
            if forbidden_subcmds.contains(first) {
                return Err(GitSecurityViolation::Forbidden(format!(
                    "forbidden git subcommand: {}",
                    joined
                )));
            }
        }

        // Forbid remote set-url
        if args.first() == Some(&"remote") && args.get(1) == Some(&"set-url") {
            return Err(GitSecurityViolation::Forbidden(format!(
                "remote set-url is not allowed: {}",
                joined
            )));
        }

        // Forbid config --global
        if args.first() == Some(&"config") && args.iter().any(|a| *a == "--global") {
            return Err(GitSecurityViolation::Forbidden(format!(
                "config --global is not allowed: {}",
                joined
            )));
        }

        Ok(())
    }

    pub async fn run(&self, args: &[&str]) -> Result<(i32, String, String)> {
        self.check_args(args).map_err(|e| anyhow!("{}", e))?;

        // Safe env: only pass a minimal allowlist
        let mut env: HashMap<String, String> = HashMap::new();
        for key in &[
            "HOME",
            "PATH",
            "USER",
            "LOGNAME",
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "GIT_SSH_COMMAND",
            "SSH_AUTH_SOCK",
        ] {
            if let Ok(val) = std::env::var(key) {
                env.insert(key.to_string(), val);
            }
        }

        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .env_clear()
            .envs(&env)
            .output()
            .await?;

        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((code, stdout, stderr))
    }

    #[allow(dead_code)]
    pub async fn run_str(&self, args: &[&str]) -> Result<String> {
        let (code, stdout, stderr) = self.run(args).await?;
        if code != 0 {
            return Err(anyhow!("git {:?} failed ({}): {}", args, code, stderr));
        }
        Ok(stdout.trim().to_string())
    }
}
