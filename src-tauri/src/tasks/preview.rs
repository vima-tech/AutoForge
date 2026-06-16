//! M5 — preview data masking (design §5.3, seed path).
//!
//! When a project declares `preview.db_path` + `preview.sensitive_fields`, this
//! snapshots the seed preview SQLite database and applies the (unit-tested)
//! field-masking engine to the snapshot before it is exposed, recording the
//! snapshot name + masking policy version on the preview environment row.
//! Best-effort: degrades to a no-op when unconfigured or sqlite3 is unavailable.

use crate::core::mask;
use crate::db::Db;
use crate::models::project::Project;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
struct PreviewYaml {
    preview: Option<PreviewSection>,
}
#[derive(Debug, Deserialize, Default)]
struct PreviewSection {
    db_path: Option<String>,
}

pub async fn apply_preview_masking(db: &Db, project: &Project, preview_id: &str, worktree_path: &str) {
    let cfg_src = crate::commands::run_config::effective_config(project);
    let Some(yaml) = cfg_src.as_deref() else {
        return;
    };
    let statements = mask::statements_from_yaml(yaml);
    if statements.is_empty() {
        return;
    }
    let cfg: PreviewYaml = serde_yaml::from_str(yaml).unwrap_or_default();
    let Some(db_rel) = cfg.preview.and_then(|p| p.db_path) else {
        return;
    };

    let src = if Path::new(&db_rel).is_absolute() {
        PathBuf::from(&db_rel)
    } else {
        Path::new(worktree_path).join(&db_rel)
    };
    if !src.exists() {
        return;
    }

    let short = &preview_id[..preview_id.len().min(8)];
    let snap_name = format!("preview_{}_{}.db", sanitize(&project.slug), short);
    let snap_path = std::env::temp_dir().join(&snap_name);
    if tokio::fs::copy(&src, &snap_path).await.is_err() {
        return;
    }

    let applied = mask::apply_to_sqlite(&snap_path.to_string_lossy(), &statements).await;
    if applied > 0 {
        let _ = sqlx::query(
            "UPDATE preview_environments
             SET db_snapshot_name=?, data_masked_at=datetime('now'), mask_policy_version=?
             WHERE id=?",
        )
        .bind(&snap_name)
        .bind(mask::MASK_POLICY_VERSION)
        .bind(preview_id)
        .execute(db)
        .await;
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

// ── M5 container preview orchestration (Podman/Docker) ──────────────────────────

#[derive(Debug, Deserialize, Default)]
struct ContainerYaml {
    preview: Option<ContainerPreview>,
}
#[derive(Debug, Deserialize, Default)]
struct ContainerPreview {
    start: Option<StartSpec>,
}
#[derive(Debug, Deserialize, Default)]
struct StartSpec {
    port: Option<u16>,
    health_check: Option<String>,
}

/// Detect an available container runtime (podman preferred, then docker).
pub fn container_runtime() -> Option<&'static str> {
    for rt in ["podman", "docker"] {
        if which(rt) {
            return Some(rt);
        }
    }
    None
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .map(|p| {
            p.split(':')
                .any(|d| std::path::Path::new(d).join(bin).exists())
        })
        .unwrap_or(false)
}

/// Deterministic host-port assignment (FNV-1a) in [19000, 23000).
fn derive_host_port(preview_id: &str) -> u16 {
    let mut h: u32 = 2166136261;
    for b in preview_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    19000 + (h % 4000) as u16
}

fn health_url(host_port: u16, health_check: &str) -> String {
    let path = health_check
        .split_whitespace()
        .last()
        .filter(|p| p.starts_with('/'))
        .unwrap_or("/");
    format!("http://127.0.0.1:{host_port}{path}")
}

/// Build + run a container preview for a worktree (best-effort). Requires a
/// Dockerfile in the worktree and an available container runtime; degrades with
/// an Err the caller ignores. On success records container_id + preview_url and
/// flips the preview to `ready` once the health check passes.
pub async fn provision_container(
    db: &Db,
    project: &Project,
    preview_id: &str,
    worktree_path: &str,
) -> Result<String, String> {
    let Some(rt) = container_runtime() else {
        return Err("无可用容器运行时(podman/docker)".into());
    };
    if !std::path::Path::new(worktree_path).join("Dockerfile").exists() {
        return Err("worktree 缺少 Dockerfile".into());
    }

    let cfg: ContainerYaml = crate::commands::run_config::effective_config(project)
        .as_deref()
        .and_then(|y| serde_yaml::from_str(y).ok())
        .unwrap_or_default();
    let start = cfg.preview.and_then(|p| p.start).unwrap_or_default();
    let container_port = start.port.unwrap_or(8000);
    let health_check = start.health_check.unwrap_or_default();
    let host_port = derive_host_port(preview_id);
    let short = &preview_id[..preview_id.len().min(12)];
    let image = format!("autoforge-preview:{short}");
    let name = format!("af-preview-{short}");

    run_cmd(rt, &["rm", "-f", &name], 30).await;
    let (bc, _, be) = run_cmd(rt, &["build", "-t", &image, worktree_path], 600).await;
    if bc != 0 {
        return Err(format!("镜像构建失败: {}", clip(&be)));
    }
    let port_map = format!("{host_port}:{container_port}");
    let (rc, _, re) = run_cmd(rt, &["run", "-d", "--name", &name, "-p", &port_map, &image], 60).await;
    if rc != 0 {
        return Err(format!("容器启动失败: {}", clip(&re)));
    }

    let url = format!("http://127.0.0.1:{host_port}");
    let health = health_url(host_port, &health_check);
    let mut ready = false;
    if let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        for _ in 0..15 {
            if client
                .get(&health)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    let _ = sqlx::query(
        "UPDATE preview_environments SET container_id=?, preview_url=?, status=? WHERE id=?",
    )
    .bind(&name)
    .bind(&url)
    .bind(if ready { "ready" } else { "building" })
    .bind(preview_id)
    .execute(db)
    .await;
    Ok(url)
}

fn clip(s: &str) -> String {
    s.chars().take(200).collect()
}

async fn run_cmd(rt: &str, args: &[&str], timeout_secs: u64) -> (i32, String, String) {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new(rt).args(args).output(),
    )
    .await;
    match out {
        Ok(Ok(o)) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Ok(Err(e)) => (-1, String::new(), e.to_string()),
        Err(_) => (-1, String::new(), "timeout".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_deterministic_and_in_range() {
        let a = derive_host_port("preview-abc-123");
        assert_eq!(a, derive_host_port("preview-abc-123"));
        assert!((19000..23000).contains(&a));
    }

    #[test]
    fn health_url_extracts_path() {
        assert_eq!(health_url(19000, "GET /health"), "http://127.0.0.1:19000/health");
        assert_eq!(health_url(20000, "/ready"), "http://127.0.0.1:20000/ready");
        assert_eq!(health_url(19000, ""), "http://127.0.0.1:19000/");
    }
}
