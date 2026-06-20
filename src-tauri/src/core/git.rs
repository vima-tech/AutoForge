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
        let deny = |reason: &str| {
            Err(GitSecurityViolation::Forbidden(format!(
                "{}: {}",
                reason, joined
            )))
        };

        // Resolve the real subcommand, skipping leading global options so callers
        // (or args) cannot hide a `push`/`config` behind `-c key=val` / `-C dir`.
        // `-c`/`-C`/`--git-dir`/`--work-tree`/`--namespace` consume the next token.
        let mut idx = 0usize;
        while idx < args.len() {
            let a = args[idx];
            // Reject `-c` overrides that can enable code execution or weaken safety.
            if a == "-c" {
                if let Some(kv) = args.get(idx + 1) {
                    let key = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
                    let dangerous = key.starts_with("alias.")
                        || key.starts_with("protocol.")
                        || key == "core.sshcommand"
                        || key == "core.fsmonitor"
                        || key == "core.pager"
                        || key == "core.editor"
                        || key == "core.hookspath"
                        || key == "uploadpack.packobjectshook"
                        || key.ends_with("helper"); // credential.*.helper, etc.
                    if dangerous {
                        return deny("dangerous -c override is not allowed");
                    }
                }
                idx += 2;
                continue;
            }
            if matches!(a, "-C" | "--git-dir" | "--work-tree" | "--namespace") {
                idx += 2;
                continue;
            }
            if a.starts_with('-') {
                idx += 1;
                continue;
            }
            break;
        }
        let subcmd = args.get(idx).copied().unwrap_or("");
        let rest = if idx < args.len() {
            &args[idx..]
        } else {
            &[][..]
        };

        match subcmd {
            "push" => {
                let push_args = &rest[1..];
                // Forbid pushes that target main/master via any refspec form.
                let touches_protected = push_args.iter().any(|a| {
                    let ref_tail = a.rsplit([':', '/']).next().unwrap_or(a);
                    ref_tail == "main" || ref_tail == "master" || *a == "main" || *a == "master"
                }) || push_args.iter().any(|a| a.contains("main") || a.contains("master"));
                if touches_protected {
                    return deny("push to main/master is not allowed");
                }
                // Forbid force / destructive push forms.
                if push_args.iter().any(|a| {
                    *a == "--force"
                        || *a == "-f"
                        || a.starts_with("--force-with-lease")
                        || a.starts_with("--force-if-includes")
                        || *a == "--mirror"
                        || *a == "--delete"
                        || *a == "-d"
                }) || push_args.iter().any(|a| a.starts_with('+'))
                {
                    return deny("force/destructive push is not allowed");
                }
            }
            "branch" => {
                if rest.iter().any(|a| *a == "-D" || *a == "-d" || *a == "--delete") {
                    // Every non-flag operand must be an autoforge/* branch.
                    let bad = rest[1..]
                        .iter()
                        .filter(|a| !a.starts_with('-'))
                        .any(|a| !a.starts_with("autoforge/"));
                    if bad {
                        return deny("branch delete only allowed for autoforge/* branches");
                    }
                }
            }
            "symbolic-ref" | "update-ref" | "fast-import" | "filter-branch" | "daemon" => {
                return deny("forbidden git subcommand");
            }
            "remote" => {
                if rest.get(1) == Some(&"set-url") || rest.get(1) == Some(&"add") {
                    return deny("modifying remotes is not allowed");
                }
            }
            "config" => {
                if rest.iter().any(|a| *a == "--global" || *a == "--system") {
                    return deny("config --global/--system is not allowed");
                }
            }
            _ => {}
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
