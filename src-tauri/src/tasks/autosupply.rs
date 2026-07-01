//! 工厂自喂料调度：周期性对活跃项目跑代码扫描 + proposer 提议，产物**全部进 triage 池**。
//!
//! 安全护栏（C4）：
//! - 永远 `IntakeMode::Triage`——绝不自动进流水线（防工厂自嗨刷需求）。
//! - 每轮总量受 `max_per_run` 限——防一次性淹没待整理池。
//! - proposer 默认关，需显式开启。
//! - 产物经 `gateway::receive` 落库，复用其去重 + `has_obvious_injection` 过滤。

use crate::core::event::{emit, AppEvent, EventSink};
use crate::db::Db;
use crate::intake::{gateway, proposer, scanner, IntakeMode};
use crate::tasks::runner::JobSender;
use std::sync::atomic::{AtomicBool, Ordering};

/// RAII 守卫：进入一轮自喂料时置位 + 广播「运行中」，离开（含 panic 提前返回）时
/// 复位 + 广播「已结束」，确保前端回显不会因异常路径而卡在「运行中」。
struct RunGuard<'a> {
    running: &'a AtomicBool,
    sink: &'a dyn EventSink,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        emit(self.sink, AppEvent::AutosupplyStatus { running: false });
    }
}

/// 自喂料配置（存 app_settings，键前缀 autosupply.*）。
#[derive(Debug, Clone)]
pub struct AutosupplyConfig {
    pub enabled: bool,
    pub interval_min: i64,
    pub scan_enabled: bool,
    pub proposer_enabled: bool,
    pub max_per_run: usize,
    /// proposer（深度提议）的**独立**每轮预算，与 scanner 的 `max_per_run` 分开计——
    /// 否则确定性 scanner 容易先打满 `max_per_run`、把高价值的 proposer 饿死。默认 8。
    pub proposer_max_per_run: usize,
    /// 严重度门槛：低于此级别的产物在入池前丢弃（low<medium<high<critical）。
    /// 默认 low（不过滤）；调高可只保留高价值问题，进一步压制 linter 噪音。
    pub min_severity: String,
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
            proposer_max_per_run: 8,
            min_severity: "low".to_string(),
            analyze_enabled: true,
            triage_enabled: true,
        }
    }
}

/// 严重度排序（用于 `min_severity` 门槛比较）。未知值按 medium 处理。
pub fn severity_rank(s: &str) -> u8 {
    match s.trim().to_ascii_lowercase().as_str() {
        "critical" => 3,
        "high" => 2,
        "low" => 0,
        _ => 1, // medium / 未知
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
            proposer_max_per_run: get_i64(db, "autosupply.proposer_max_per_run").await.unwrap_or(d.proposer_max_per_run as i64).clamp(1, 50) as usize,
            min_severity: get_setting(db, "autosupply.min_severity").await.unwrap_or(d.min_severity),
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
async fn set_setting(db: &Db, key: &str, value: &str) {
    let _ = sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await;
}

/// 当前 Unix 时间戳（秒）。
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 上次自喂料**实际运行**的 Unix 时间戳（秒）；从未运行过返回 `None`。
/// 调度器据此按"距上次运行已过多久"计算下次触发，使频繁重启（dev 热重载）
/// 不再清零计时器。手动「立即运行」与定时触发共用此戳。
pub async fn last_run_unix(db: &Db) -> Option<i64> {
    get_i64(db, "autosupply.last_run_at").await
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
    sink: &dyn EventSink,
    cfg: &AutosupplyConfig,
    running: &AtomicBool,
) -> CycleStats {
    // 已有一轮在跑（手动与周期调度共用此标志）→ 直接返回，避免并发重复供料。
    if running.swap(true, Ordering::SeqCst) {
        return CycleStats::default();
    }
    emit(sink, AppEvent::AutosupplyStatus { running: true });
    let _guard = RunGuard { running, sink };

    let projects = sqlx::query_as::<_, (String, String)>(
        "SELECT id, repo_path FROM projects WHERE status='active'",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut stats = CycleStats::default();
    // 本轮**新入池**的 triage 条目 id，供前置整理去噪。
    let mut fresh_ids: Vec<String> = Vec::new();
    // 两类来源**各自独立**计预算：scanner 用 max_per_run，proposer 用 proposer_max_per_run。
    // 关键：只有真正 INSERT 的全新条目才计入预算（去重命中不计），否则重复的 linter 条目
    // 每轮把预算占满，既挡住新发现、也饿死高价值的 proposer。
    let min_rank = severity_rank(&cfg.min_severity);
    let mut scanned_new = 0usize;
    let mut proposed_new = 0usize;

    for (pid, repo_path) in projects {
        if scanned_new >= cfg.max_per_run
            && (!cfg.proposer_enabled || proposed_new >= cfg.proposer_max_per_run)
        {
            break;
        }

        if cfg.scan_enabled && !repo_path.is_empty() && scanned_new < cfg.max_per_run {
            let mut payloads = scanner::scan_todos(&pid, &repo_path).await;
            // 半成品/未实现桩：直击「功能只做了一半」的强信号（linter 覆盖不到）。
            payloads.extend(scanner::scan_unfinished(&pid, &repo_path).await);
            payloads.extend(scanner::scan_cargo_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_npm_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_pip_audit(&pid, &repo_path).await);
            payloads.extend(scanner::scan_govulncheck(&pid, &repo_path).await);
            // 静态代码分析：发现真实代码问题（clippy/ruff/go vet/eslint），按栈自动调度。
            if cfg.analyze_enabled {
                payloads.extend(scanner::scan_static_analysis(&pid, &repo_path).await);
            }
            for p in payloads {
                if scanned_new >= cfg.max_per_run {
                    break;
                }
                // 严重度门槛：低于阈值的产物直接丢弃，不入池。
                if severity_rank(p.severity.as_deref().unwrap_or("medium")) < min_rank {
                    continue;
                }
                // 安全护栏：永远 Triage。只有真正新建才计预算 + 计入待整理。
                if let Ok((issue, created)) =
                    gateway::receive_with_status(db, job_tx, sink, p, IntakeMode::Triage).await
                {
                    if created {
                        stats.scanned += 1;
                        scanned_new += 1;
                        if issue.status == "triage" {
                            fresh_ids.push(issue.id);
                        }
                    }
                }
            }
        }

        if cfg.proposer_enabled && proposed_new < cfg.proposer_max_per_run {
            let remaining = cfg.proposer_max_per_run.saturating_sub(proposed_new);
            if let Ok(payloads) = proposer::propose(db, &pid, remaining).await {
                for p in payloads {
                    if proposed_new >= cfg.proposer_max_per_run {
                        break;
                    }
                    if severity_rank(p.severity.as_deref().unwrap_or("medium")) < min_rank {
                        continue;
                    }
                    if let Ok((issue, created)) =
                        gateway::receive_with_status(db, job_tx, sink, p, IntakeMode::Triage).await
                    {
                        if created {
                            stats.proposed += 1;
                            proposed_new += 1;
                            if issue.status == "triage" {
                                fresh_ids.push(issue.id);
                            }
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

    // proposer 健康巡检：连续多轮失败 → 进收件箱提醒（避免深度提议器静默空转无人知）。
    if cfg.proposer_enabled {
        check_proposer_health(db, sink).await;
    }

    // 记录本轮运行时间：下次调度据此按真实间隔计算，重启不再清零计时器。
    // 仅真实运行的路径会到达这里（已在跑的并发轮在函数开头就提前返回）。
    set_setting(db, "autosupply.last_run_at", &now_unix().to_string()).await;

    stats
}

/// proposer 健康巡检：最近多次结构化产出全部失败（status=error，多为模型工具调用格式不兼容
/// 或 proposer 未绑定可用 LLM）→ 发 `AutosupplyDegraded` 进收件箱提醒操作者。
/// 通知按 thread_key 折叠为一行，不会逐轮刷屏。
async fn check_proposer_health(db: &Db, sink: &dyn EventSink) {
    const WINDOW: i64 = 6; // 巡检最近 6 次 proposer 产出
    const MIN_STREAK: usize = 3; // 至少连续 3 次失败才告警，避免偶发抖动误报
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT status FROM agent_outputs WHERE role='proposer' ORDER BY created_at DESC LIMIT ?",
    )
    .bind(WINDOW)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    if rows.len() >= MIN_STREAK && rows.iter().all(|s| s == "error") {
        emit(
            sink,
            AppEvent::AutosupplyDegraded {
                reason: "最近多轮结构化产出解析失败（status=error）。常见原因：所选模型的工具调用格式不被支持，或 proposer 角色未绑定可用 LLM。建议改用支持原生 function-calling 的强推理模型。"
                    .to_string(),
            },
        );
    }
}
