//! 工厂自喂料调度：周期性对活跃项目跑代码扫描 + proposer 提议，产物**全部进 triage 池**。
//!
//! 安全护栏（C4）：
//! - 永远 `IntakeMode::Triage`——绝不自动进流水线（防工厂自嗨刷需求）。
//! - 每轮总量受 `max_per_run` 限——防一次性淹没待整理池。
//! - proposer 默认关，需显式开启。
//! - 产物经 `gateway::receive` 落库，复用其去重 + `has_obvious_injection` 过滤。

use crate::db::Db;
use crate::intake::{gateway, proposer, scanner, IntakeMode};
use crate::tasks::runner::JobSender;
use tauri::AppHandle;

/// 自喂料配置（存 app_settings，键前缀 autosupply.*）。
#[derive(Debug, Clone)]
pub struct AutosupplyConfig {
    pub enabled: bool,
    pub interval_min: i64,
    pub scan_enabled: bool,
    pub proposer_enabled: bool,
    pub max_per_run: usize,
}

impl Default for AutosupplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_min: 1440,
            scan_enabled: true,
            proposer_enabled: false,
            max_per_run: 20,
        }
    }
}

impl AutosupplyConfig {
    pub async fn load(db: &Db) -> Self {
        let d = Self::default();
        Self {
            enabled: get_bool(db, "autosupply.enabled").await.unwrap_or(d.enabled),
            interval_min: get_i64(db, "autosupply.interval_min").await.unwrap_or(d.interval_min).max(5),
            scan_enabled: get_bool(db, "autosupply.scan_enabled").await.unwrap_or(d.scan_enabled),
            proposer_enabled: get_bool(db, "autosupply.proposer_enabled").await.unwrap_or(d.proposer_enabled),
            max_per_run: get_i64(db, "autosupply.max_per_run").await.unwrap_or(d.max_per_run as i64).clamp(1, 200) as usize,
        }
    }
}

async fn get_setting(db: &Db, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}
async fn get_bool(db: &Db, key: &str) -> Option<bool> {
    get_setting(db, key).await.map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
}
async fn get_i64(db: &Db, key: &str) -> Option<i64> {
    get_setting(db, key).await.and_then(|s| s.trim().parse::<i64>().ok())
}

/// 一轮自喂料：对所有活跃项目跑扫描 + proposer，全部 mode=Triage。
/// 返回 (扫描入池数, 提议入池数)。
pub async fn run_cycle(
    db: &Db,
    job_tx: &JobSender,
    app: &AppHandle,
    cfg: &AutosupplyConfig,
) -> (u32, u32) {
    let projects = sqlx::query_as::<_, (String, String)>(
        "SELECT id, repo_path FROM projects WHERE status='active'",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut total = 0usize;
    let mut scanned = 0u32;
    let mut proposed = 0u32;

    for (pid, repo_path) in projects {
        if total >= cfg.max_per_run {
            break;
        }

        if cfg.scan_enabled && !repo_path.is_empty() {
            let mut payloads = scanner::scan_todos(&pid, &repo_path).await;
            payloads.extend(scanner::scan_cargo_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_npm_audit(&pid, &repo_path).await);
            for p in payloads {
                if total >= cfg.max_per_run {
                    break;
                }
                // 安全护栏：永远 Triage。
                if gateway::receive(db, job_tx, app, p, IntakeMode::Triage).await.is_ok() {
                    scanned += 1;
                    total += 1;
                }
            }
        }

        if cfg.proposer_enabled && total < cfg.max_per_run {
            let remaining = cfg.max_per_run.saturating_sub(total).min(8);
            if let Ok(payloads) = proposer::propose(db, &pid, remaining).await {
                for p in payloads {
                    if total >= cfg.max_per_run {
                        break;
                    }
                    if gateway::receive(db, job_tx, app, p, IntakeMode::Triage).await.is_ok() {
                        proposed += 1;
                        total += 1;
                    }
                }
            }
        }
    }

    (scanned, proposed)
}
