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
    /// 静态代码分析（clippy/ruff/go vet/eslint），发现真实代码问题。默认开。
    pub analyze_enabled: bool,
    /// 前置整理：入池后立即跑 triage Agent 滤掉噪音、就地归一化幸存条目（仍留 triage 池）。
    /// 默认开——让人工闸口只看到干净条目，不必先点「整理」再清噪。
    pub triage_enabled: bool,
}

impl Default for AutosupplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_min: 1440,
            scan_enabled: true,
            proposer_enabled: false,
            max_per_run: 20,
            analyze_enabled: true,
            triage_enabled: true,
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
            analyze_enabled: get_bool(db, "autosupply.analyze_enabled").await.unwrap_or(d.analyze_enabled),
            triage_enabled: get_bool(db, "autosupply.triage_enabled").await.unwrap_or(d.triage_enabled),
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

/// 一轮自喂料的统计。
#[derive(Debug, Default, Clone, Copy)]
pub struct CycleStats {
    /// 扫描器（TODO + 依赖审计 + 静态分析）入池数。
    pub scanned: u32,
    /// proposer 提议入池数。
    pub proposed: u32,
    /// 前置整理判为噪音、已丢弃的条数。
    pub discarded: u32,
}

/// 一轮自喂料：对所有活跃项目跑扫描（TODO + 依赖审计 + 静态代码分析）+ proposer，
/// 全部 mode=Triage（安全护栏 C4：永不自动进流水线）；入池后按配置**前置整理**——
/// triage Agent 立即滤掉噪音、就地归一化幸存条目（仍留 triage 池等人工闸口）。
pub async fn run_cycle(
    db: &Db,
    job_tx: &JobSender,
    app: &AppHandle,
    cfg: &AutosupplyConfig,
) -> CycleStats {
    let projects = sqlx::query_as::<_, (String, String)>(
        "SELECT id, repo_path FROM projects WHERE status='active'",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut total = 0usize;
    let mut stats = CycleStats::default();
    // 本轮新入池（或命中已有 triage 条目）的 id，供前置整理去噪。
    let mut fresh_ids: Vec<String> = Vec::new();

    for (pid, repo_path) in projects {
        if total >= cfg.max_per_run {
            break;
        }

        if cfg.scan_enabled && !repo_path.is_empty() {
            let mut payloads = scanner::scan_todos(&pid, &repo_path).await;
            payloads.extend(scanner::scan_cargo_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_npm_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_pip_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_govulncheck(&pid, &repo_path).await);
            // 静态代码分析：发现真实代码问题（clippy/ruff/go vet/eslint），按栈自动调度。
            if cfg.analyze_enabled {
                payloads.extend(scanner::scan_static_analysis(&pid, &repo_path).await);
            }
            for p in payloads {
                if total >= cfg.max_per_run {
                    break;
                }
                // 安全护栏：永远 Triage。
                if let Ok(issue) = gateway::receive(db, job_tx, app, p, IntakeMode::Triage).await {
                    stats.scanned += 1;
                    total += 1;
                    if issue.status == "triage" {
                        fresh_ids.push(issue.id);
                    }
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
                    if let Ok(issue) = gateway::receive(db, job_tx, app, p, IntakeMode::Triage).await {
                        stats.proposed += 1;
                        total += 1;
                        if issue.status == "triage" {
                            fresh_ids.push(issue.id);
                        }
                    }
                }
            }
        }
    }

    // 前置整理：入池即去噪 + 归一化，幸存条目仍留 triage 池（不进流水线）。
    if cfg.triage_enabled && !fresh_ids.is_empty() {
        let denoise = crate::intake::triage::denoise_in_place(db, fresh_ids).await;
        stats.discarded = denoise.discarded;
    }

    stats
}
