use crate::agents::local_claude;
use crate::models::{
    admin_decision::AdminDecision,
    preview::PreviewEnvironment,
    test_session::{ScanFinding, TestSession},
};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;
use tracing::{debug, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemHealth {
    pub status: String,
    pub db_ok: bool,
    pub claude_auth: bool,
    pub version: String,
    pub active_slots: usize,
    pub max_slots: usize,
    pub total_slot_capacity: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SlotOccupant {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectSlotStats {
    pub project_id: String,
    pub project_name: String,
    pub project_status: String,
    pub active_slots: usize,
    pub max_slots: usize,
    pub executing_slots: usize,
    pub pending_review_slots: usize,
    pub occupants: Vec<SlotOccupant>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectPipelineStats {
    pub project_id: String,
    pub project_name: String,
    pub project_status: String,
    pub triage: i64,
    pub pending_analysis: i64,
    pub pending_review_1: i64,
    pub executing: i64,
    pub pending_review_2: i64,
    pub merged: i64,
    pub rejected: i64,
    pub total_issues: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineStats {
    pub triage: i64,
    pub pending_analysis: i64,
    pub pending_review_1: i64,
    pub executing: i64,
    pub pending_review_2: i64,
    pub merged: i64,
    pub rejected: i64,
    pub total_issues: i64,
    pub active_projects: i64,
    pub active_slots: usize,
    pub max_slots: usize,
    pub total_slot_capacity: usize,
    pub pending_review_slots: usize,
    pub pause_threshold: usize,
    pub stage: String,
    pub executing_cr_ids: Vec<String>,
    pub project_slots: Vec<ProjectSlotStats>,
    pub project_pipelines: Vec<ProjectPipelineStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateConcurrencyConfig {
    pub max_slots: Option<usize>,
    pub pause_threshold: Option<usize>,
    pub queue_strategy: Option<String>,
    /// 代码 agent 墙钟超时（分钟）。
    pub timeout_min: Option<u64>,
    /// 代码 agent 空闲超时（分钟，0 = 关闭）。
    pub idle_timeout_min: Option<u64>,
    /// 负载感知入场：系统 1 分钟负载 > factor×nproc 时暂缓启动新 agent（0 = 关闭）。
    pub max_load_factor: Option<f64>,
    /// 合并门构建池：任意时刻最多并发的编译/测试数。
    pub build_slots: Option<usize>,
    /// cgroup CPU 预算（占总核数百分比，0 = 关闭；仅 Linux 生效）。
    pub cpu_budget_pct: Option<u64>,
    /// 出站 LLM 并发上限：同时打到 LLM 服务商的请求数（防 429 限流）。
    pub llm_max_concurrency: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    pub active_slots: usize,
    pub max_slots: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
    pub queue_strategy: String,
    /// 代码 agent 墙钟超时（分钟）。
    pub timeout_min: u64,
    /// 代码 agent 空闲超时（分钟，0 = 关闭）。
    pub idle_timeout_min: u64,
    /// 负载感知入场阈值（factor×nproc，0 = 关闭）。
    pub max_load_factor: f64,
    /// 合并门构建池大小。
    pub build_slots: usize,
    /// cgroup CPU 预算（% × nproc，0 = 关闭）。
    pub cpu_budget_pct: u64,
    /// 出站 LLM 并发上限（防 429 限流）。
    pub llm_max_concurrency: usize,
}

/// 代码 agent 超时默认值（分钟）。墙钟是硬上限兜底，空闲超时是抓卡死的主闸。
pub const DEFAULT_TIMEOUT_MIN: u64 = 30;
/// 空闲超时默认值按平台走：Linux 有 CPU 感知判定（仍烧 CPU 不算卡死、不误杀安静长构建），
/// 故默认 8 分钟开启；非 Linux 无 /proc 读 CPU，空闲只能"看输出"，为避免误杀不流式的长构建，
/// 默认 0（关闭），由墙钟兜底，用户可手动开。
#[cfg(target_os = "linux")]
pub const DEFAULT_IDLE_TIMEOUT_MIN: u64 = 8;
#[cfg(not(target_os = "linux"))]
pub const DEFAULT_IDLE_TIMEOUT_MIN: u64 = 0;
/// 负载感知入场默认阈值：负载 > 1.5×nproc 才暂缓——只在真过载时踩刹车，正常不挡。
pub const DEFAULT_MAX_LOAD_FACTOR: f64 = 1.5;
/// 合并门构建池默认并发：2 个编译/测试同时跑（每个可吃多核，故不宜大）。
pub const DEFAULT_BUILD_SLOTS: usize = 2;
/// CPU 预算默认 0=关（cgroup 依环境，显式开启更安全；建议 Linux 上设 70~80）。
pub const DEFAULT_CPU_BUDGET_PCT: u64 = 0;
/// 出站 LLM 并发上限默认值：限制同时打到 LLM 服务商的请求数，防批量任务（如一次分析 50 条
/// 需求）瞬间数十并发触发 429 限流。保守取 4，可在「并发控制」按服务商配额调高。
pub const DEFAULT_LLM_CONCURRENCY: usize = 4;

/// 读取合并门构建池大小（clamp [1, 32]）。
pub async fn load_build_slots(db: &crate::db::Db) -> usize {
    let v: Option<(String,)> =
        sqlx::query_as("SELECT value FROM app_settings WHERE key='execution.build_slots'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    v.and_then(|(s,)| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_BUILD_SLOTS)
        .clamp(1, 32)
}

/// 读取 CPU 预算百分比（clamp [0, 100]，0=关）。
pub async fn load_cpu_budget_pct(db: &crate::db::Db) -> u64 {
    let v: Option<(String,)> =
        sqlx::query_as("SELECT value FROM app_settings WHERE key='execution.cpu_budget_pct'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    v.and_then(|(s,)| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CPU_BUDGET_PCT)
        .min(100)
}

/// 读取出站 LLM 并发上限（clamp [1, 32]）。供启动初始化与设置热更新用。
pub async fn load_llm_concurrency(db: &crate::db::Db) -> usize {
    let v: Option<(String,)> =
        sqlx::query_as("SELECT value FROM app_settings WHERE key='llm.max_concurrency'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    v.and_then(|(s,)| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LLM_CONCURRENCY)
        .clamp(1, 32)
}

/// 读取负载感知入场阈值（factor×nproc）。0 或负 = 关闭。clamp 到 [0, 8]。
pub async fn load_max_load_factor(db: &crate::db::Db) -> f64 {
    let v: Option<(String,)> =
        sqlx::query_as("SELECT value FROM app_settings WHERE key='execution.max_load_factor'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let f = v
        .and_then(|(s,)| s.parse::<f64>().ok())
        .unwrap_or(DEFAULT_MAX_LOAD_FACTOR);
    f.clamp(0.0, 8.0)
}

/// 读取并 clamp 代码 agent 超时设置，返回 (墙钟秒, 空闲秒)。供 `tasks::execution` 用。
/// 墙钟 clamp 到 [5, 180] 分钟；空闲 clamp 到 [0, 60] 分钟（0=关闭）。空闲若 > 墙钟则取墙钟。
pub async fn load_execution_limits(db: &crate::db::Db) -> (u64, u64) {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings
         WHERE key IN ('execution.timeout_min', 'execution.idle_timeout_min')",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut wall_min = DEFAULT_TIMEOUT_MIN;
    let mut idle_min = DEFAULT_IDLE_TIMEOUT_MIN;
    for (key, value) in rows {
        match key.as_str() {
            "execution.timeout_min" => {
                wall_min = value.parse::<u64>().unwrap_or(DEFAULT_TIMEOUT_MIN);
            }
            "execution.idle_timeout_min" => {
                idle_min = value.parse::<u64>().unwrap_or(DEFAULT_IDLE_TIMEOUT_MIN);
            }
            _ => {}
        }
    }
    let wall_min = wall_min.clamp(5, 180);
    // 空闲超时不得超过墙钟，否则永不先于墙钟触发，等于无效。
    let idle_min = idle_min.min(60).min(wall_min);
    (wall_min * 60, idle_min * 60)
}

pub async fn load_concurrency_settings(
    db: &crate::db::Db,
) -> Result<(usize, usize, String), String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings
         WHERE key IN ('concurrency.max_slots', 'concurrency.pause_threshold', 'concurrency.queue_strategy')",
    )
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    let default_slots = 5;
    let mut max_slots = default_slots;
    let mut pause_threshold = 20;
    let mut queue_strategy = "priority".to_string();

    for (key, value) in rows {
        match key.as_str() {
            "concurrency.max_slots" => {
                max_slots = value.parse::<usize>().unwrap_or(default_slots).max(1);
            }
            "concurrency.pause_threshold" => {
                pause_threshold = value.parse::<usize>().unwrap_or(20).max(1);
            }
            "concurrency.queue_strategy" => {
                queue_strategy = match value.as_str() {
                    "fifo" | "priority" | "oldest" => value,
                    _ => "priority".to_string(),
                };
            }
            _ => {}
        }
    }

    Ok((max_slots, pause_threshold, queue_strategy))
}

#[tauri::command]
pub async fn system_health(state: State<'_, AppState>) -> Result<SystemHealth, String> {
    let t0 = std::time::Instant::now();
    debug!("[cmd] system_health start");
    // Check DB
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    // Auth check is intentionally NOT done here.
    // Spawning the claude Electron subprocess from within a WebKitGTK process
    // delivers SIGTRAP to the parent via kill(getppid(), SIGTRAP), which
    // triggers a NeedDebuggerBreak trap that permanently freezes the GTK event
    // loop and drops all subsequent Tauri IPC calls.
    // Use the dedicated `check_claude_auth` command for a one-shot lazy check.
    let claude_auth = true;

    let pipeline_status = state.concurrency.status();
    let (executing,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='executing'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((pipeline_status.active_slots as i64,));
    let (pending_review,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM change_requests WHERE status='pending_code_review'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((pipeline_status.pending_review as i64,));
    let (active_projects,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status='active'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((1,));
    let stage = if pending_review as usize >= pipeline_status.pause_threshold {
        "paused".to_string()
    } else if pending_review as usize >= pipeline_status.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    info!("[cmd] system_health done in {:?}", t0.elapsed());
    Ok(SystemHealth {
        status: if db_ok { "ok" } else { "degraded" }.to_string(),
        db_ok,
        claude_auth,
        version: env!("CARGO_PKG_VERSION").to_string(),
        active_slots: (executing + pending_review) as usize,
        max_slots: pipeline_status.max_slots,
        total_slot_capacity: (active_projects as usize).max(1) * pipeline_status.max_slots,
        pending_review: pending_review as usize,
        pause_threshold: pipeline_status.pause_threshold,
        stage,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemResources {
    /// 整机 CPU 占用百分比（0–100，所有核心平均）。
    pub cpu_pct: f32,
    /// 内存占用百分比（0–100）。
    pub mem_pct: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
}

/// 常驻 System 实例：CPU 占用是「两次采样之间的增量」，必须复用同一实例才能算出
/// 真实利用率。前端按固定间隔轮询本命令，每次返回的就是「距上次轮询」的 CPU 占用。
static SYS_MONITOR: std::sync::OnceLock<std::sync::Mutex<sysinfo::System>> =
    std::sync::OnceLock::new();

/// 标题栏系统资源监视：返回当前整机 CPU / 内存占用。跨平台（Linux/macOS/Windows），
/// 与 Tauri 无关，仅读 /proc 等系统接口，开销极小。
#[tauri::command]
pub async fn system_resources() -> Result<SystemResources, String> {
    let lock = SYS_MONITOR.get_or_init(|| std::sync::Mutex::new(sysinfo::System::new()));
    let mut sys = lock.lock().map_err(|e| e.to_string())?;
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let mem_total = sys.total_memory();
    let mem_used = sys.used_memory();
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64 * 100.0) as f32
    } else {
        0.0
    };
    Ok(SystemResources {
        cpu_pct: sys.global_cpu_usage(),
        mem_pct,
        mem_used_mb: mem_used / 1024 / 1024,
        mem_total_mb: mem_total / 1024 / 1024,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BadgeCounts {
    pub chat_unread: i64,
    pub audit_pending: i64,
}

/// Lazily check claude CLI auth. Called once from the frontend with a long
/// delay so the subprocess never runs during the critical startup window.
#[tauri::command]
pub async fn check_claude_auth() -> Result<bool, String> {
    Ok(local_claude::check_auth().await)
}

#[tauri::command]
pub async fn get_badge_counts(state: State<'_, AppState>) -> Result<BadgeCounts, String> {
    debug!("[cmd] get_badge_counts start");
    let (chat_unread,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM messages m
         LEFT JOIN conversation_reads r ON r.conversation_id = m.conversation_id
         WHERE m.from_agent IS NOT NULL
           AND m.created_at > COALESCE(r.read_at, '1970-01-01')",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // 功能审计页统一管理两个审核节点：需求审核（issues.pending_issue_review）+ 代码审核（change_requests.pending_code_review）
    let (audit_pending,): (i64,) = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM change_requests WHERE status='pending_code_review')
         + (SELECT COUNT(*) FROM issues WHERE status='pending_issue_review')",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    debug!(
        "[cmd] get_badge_counts done: chat={} audit={}",
        chat_unread, audit_pending
    );
    Ok(BadgeCounts {
        chat_unread,
        audit_pending,
    })
}

#[tauri::command]
pub async fn pipeline_stats(state: State<'_, AppState>) -> Result<PipelineStats, String> {
    let t0 = std::time::Instant::now();
    debug!("[cmd] pipeline_stats start");
    let concurrency = state.concurrency.status();

    // Batch all issue status counts into a single query.
    let (triage, pending_analysis, pending_review_1, executing_issues, rejected_issues, total_issues): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT
           SUM(CASE WHEN status='triage'             THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='pending_analysis'  THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='pending_issue_review'  THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='executing'          THEN 1 ELSE 0 END),
           SUM(CASE WHEN status='rejected'           THEN 1 ELSE 0 END),
           SUM(CASE WHEN status != 'triage'          THEN 1 ELSE 0 END)
         FROM issues",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Batch all change_request status counts into a single query.
    let (executing_crs, pending_review_2, merged, rejected_crs): (i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT
               SUM(CASE WHEN status='executing'          THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='pending_code_review'   THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='merged'             THEN 1 ELSE 0 END),
               SUM(CASE WHEN status='rejected'           THEN 1 ELSE 0 END)
             FROM change_requests",
        )
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let (active_projects,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status='active'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let executing_cr_ids = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM change_requests WHERE status='executing' ORDER BY updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|(id,)| id)
    .collect::<Vec<_>>();

    let project_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, name, status FROM projects WHERE status='active' ORDER BY name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let occupied_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT cr.project_id, cr.id, cr.status
         FROM change_requests cr
         JOIN projects p ON p.id = cr.project_id
         WHERE p.status='active' AND cr.status IN ('executing', 'pending_code_review')
         ORDER BY cr.updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut occupants_by_project: HashMap<String, Vec<SlotOccupant>> = HashMap::new();
    for (project_id, id, status) in occupied_rows {
        occupants_by_project
            .entry(project_id)
            .or_default()
            .push(SlotOccupant { id, status });
    }

    let issue_pipeline_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, i64, i64)>(
        "SELECT project_id,
                SUM(CASE WHEN status='triage' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='pending_analysis' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='pending_issue_review' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='executing' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status != 'triage' THEN 1 ELSE 0 END)
         FROM issues
         GROUP BY project_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let cr_pipeline_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT project_id,
                SUM(CASE WHEN status='executing' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='pending_code_review' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='merged' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END)
         FROM change_requests
         GROUP BY project_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let issue_pipeline_by_project = issue_pipeline_rows
        .into_iter()
        .map(
            |(
                project_id,
                triage,
                pending_analysis,
                pending_review_1,
                executing,
                rejected,
                total_issues,
            )| {
                (
                    project_id,
                    (
                        triage,
                        pending_analysis,
                        pending_review_1,
                        executing,
                        rejected,
                        total_issues,
                    ),
                )
            },
        )
        .collect::<HashMap<_, _>>();

    let cr_pipeline_by_project = cr_pipeline_rows
        .into_iter()
        .map(
            |(project_id, executing, pending_review_2, merged, rejected)| {
                (project_id, (executing, pending_review_2, merged, rejected))
            },
        )
        .collect::<HashMap<_, _>>();

    let project_slots = project_rows
        .iter()
        .map(|(project_id, project_name, project_status)| {
            let occupants = occupants_by_project.remove(project_id).unwrap_or_default();
            let executing_slots = occupants
                .iter()
                .filter(|slot| slot.status == "executing")
                .count();
            let pending_review_slots = occupants
                .iter()
                .filter(|slot| slot.status == "pending_code_review")
                .count();

            ProjectSlotStats {
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                project_status: project_status.clone(),
                active_slots: occupants.len(),
                max_slots: concurrency.max_slots,
                executing_slots,
                pending_review_slots,
                occupants,
            }
        })
        .collect::<Vec<_>>();

    let project_pipelines = project_rows
        .iter()
        .map(|(project_id, project_name, project_status)| {
            let (
                issue_triage,
                issue_pending_analysis,
                issue_pending_review_1,
                issue_executing,
                issue_rejected,
                project_total_issues,
            ) = issue_pipeline_by_project
                .get(project_id)
                .copied()
                .unwrap_or((0, 0, 0, 0, 0, 0));
            let (cr_executing, cr_pending_review_2, cr_merged, cr_rejected) =
                cr_pipeline_by_project
                    .get(project_id)
                    .copied()
                    .unwrap_or((0, 0, 0, 0));

            ProjectPipelineStats {
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                project_status: project_status.clone(),
                triage: issue_triage,
                pending_analysis: issue_pending_analysis,
                pending_review_1: issue_pending_review_1,
                executing: issue_executing.max(cr_executing),
                pending_review_2: cr_pending_review_2,
                merged: cr_merged,
                rejected: issue_rejected.max(cr_rejected),
                total_issues: project_total_issues,
            }
        })
        .collect::<Vec<_>>();

    let total_slot_capacity = project_slots.len().max(1) * concurrency.max_slots;
    let active_slots: usize = project_slots
        .iter()
        .map(|project| project.active_slots)
        .sum();
    let pending_review_slots = pending_review_2.max(concurrency.pending_review as i64) as usize;
    let stage = if pending_review_slots >= concurrency.pause_threshold {
        "paused".to_string()
    } else if pending_review_slots >= concurrency.pause_threshold / 2 {
        "throttled".to_string()
    } else {
        "normal".to_string()
    };

    let result = Ok(PipelineStats {
        triage,
        pending_analysis,
        pending_review_1,
        executing: executing_crs.max(executing_issues),
        pending_review_2,
        merged,
        rejected: rejected_crs.max(rejected_issues),
        total_issues,
        active_projects,
        active_slots,
        max_slots: concurrency.max_slots,
        total_slot_capacity,
        pending_review_slots,
        pause_threshold: concurrency.pause_threshold,
        stage,
        executing_cr_ids,
        project_slots,
        project_pipelines,
    });
    info!("[cmd] pipeline_stats done in {:?}", t0.elapsed());
    result
}

#[tauri::command]
pub async fn update_concurrency_config(
    payload: UpdateConcurrencyConfig,
    state: State<'_, AppState>,
) -> Result<ConcurrencyConfig, String> {
    let status = state.concurrency.update_config(
        payload.max_slots,
        payload.pause_threshold,
        payload.queue_strategy,
    );
    let queue_strategy = state.concurrency.queue_strategy();

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.max_slots', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(status.max_slots.to_string())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.pause_threshold', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(status.pause_threshold.to_string())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('concurrency.queue_strategy', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
    )
    .bind(&queue_strategy)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // 代码 agent 超时（可选）。clamp 与 load_execution_limits 保持一致。
    if let Some(t) = payload.timeout_min {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('execution.timeout_min', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(t.clamp(5, 180).to_string())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(i) = payload.idle_timeout_min {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('execution.idle_timeout_min', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(i.min(60).to_string())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(f) = payload.max_load_factor {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('execution.max_load_factor', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(format!("{:.2}", f.clamp(0.0, 8.0)))
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(b) = payload.build_slots {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('execution.build_slots', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(b.clamp(1, 32).to_string())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        // 即时调整构建池容量（与 max_slots/cpu_budget 一致，无需重启）。
        crate::state::set_build_slots(b.clamp(1, 32));
    }
    if let Some(p) = payload.cpu_budget_pct {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('execution.cpu_budget_pct', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(p.min(100).to_string())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        // 热改 cgroup 预算（仅在已启用时生效；0 → 解除限制 max）。
        crate::core::cpubudget::set_budget(p.min(100));
    }
    if let Some(c) = payload.llm_max_concurrency {
        let n = c.clamp(1, 32);
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES ('llm.max_concurrency', ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(n.to_string())
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        // 热更新出站 LLM 并发闸（无需重启）。
        crate::agents::llm::set_llm_concurrency(n);
    }

    let (wall_secs, idle_secs) = load_execution_limits(&state.db).await;
    let max_load_factor = load_max_load_factor(&state.db).await;
    let build_slots = load_build_slots(&state.db).await;
    let cpu_budget_pct = load_cpu_budget_pct(&state.db).await;
    let llm_max_concurrency = load_llm_concurrency(&state.db).await;
    Ok(ConcurrencyConfig {
        active_slots: status.active_slots,
        max_slots: status.max_slots,
        pending_review: status.pending_review,
        pause_threshold: status.pause_threshold,
        stage: status.stage,
        queue_strategy,
        timeout_min: wall_secs / 60,
        idle_timeout_min: idle_secs / 60,
        max_load_factor,
        build_slots,
        cpu_budget_pct,
        llm_max_concurrency,
    })
}

#[tauri::command]
pub async fn get_concurrency_config(
    state: State<'_, AppState>,
) -> Result<ConcurrencyConfig, String> {
    let status = state.concurrency.status();
    let (wall_secs, idle_secs) = load_execution_limits(&state.db).await;
    let max_load_factor = load_max_load_factor(&state.db).await;
    let build_slots = load_build_slots(&state.db).await;
    let cpu_budget_pct = load_cpu_budget_pct(&state.db).await;
    let llm_max_concurrency = load_llm_concurrency(&state.db).await;

    Ok(ConcurrencyConfig {
        active_slots: status.active_slots,
        max_slots: status.max_slots,
        pending_review: status.pending_review,
        pause_threshold: status.pause_threshold,
        stage: status.stage,
        queue_strategy: state.concurrency.queue_strategy(),
        timeout_min: wall_secs / 60,
        idle_timeout_min: idle_secs / 60,
        max_load_factor,
        build_slots,
        cpu_budget_pct,
        llm_max_concurrency,
    })
}

#[tauri::command]
pub async fn list_preview_environments(
    project_id: Option<String>,
    status: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PreviewEnvironment>, String> {
    match (project_id, status) {
        (Some(project_id), Some(status)) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE project_id=? AND status=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .bind(status)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (Some(project_id), None) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (None, Some(status)) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments WHERE status=? ORDER BY created_at DESC",
        )
        .bind(status)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
        (None, None) => sqlx::query_as::<_, PreviewEnvironment>(
            "SELECT * FROM preview_environments ORDER BY created_at DESC",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn list_test_sessions(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<TestSession>, String> {
    if let Some(project_id) = project_id {
        sqlx::query_as::<_, TestSession>(
            "SELECT * FROM test_sessions WHERE project_id=? ORDER BY COALESCE(started_at, completed_at) DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, TestSession>(
            "SELECT * FROM test_sessions ORDER BY COALESCE(started_at, completed_at) DESC",
        )
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn list_scan_findings(
    test_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ScanFinding>, String> {
    if let Some(test_session_id) = test_session_id {
        sqlx::query_as::<_, ScanFinding>(
            "SELECT * FROM scan_findings WHERE test_session_id=? ORDER BY created_at DESC",
        )
        .bind(test_session_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, ScanFinding>("SELECT * FROM scan_findings ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

/// One failed background job, for the Settings 错误历史 panel. Surfaces the
/// `last_error` the runner already persists so failures are inspectable in-app
/// instead of only in stderr logs.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct JobFailure {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub attempt: i64,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Recent failed jobs (most recent first). Capped to avoid unbounded payloads.
#[tauri::command]
pub async fn list_job_failures(
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<JobFailure>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    sqlx::query_as::<_, JobFailure>(
        "SELECT id, job_type, status, attempt, last_error, updated_at
         FROM job_executions WHERE status='failed' ORDER BY updated_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_admin_decisions(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<AdminDecision>, String> {
    if let Some(project_id) = project_id {
        sqlx::query_as::<_, AdminDecision>(
            "SELECT * FROM admin_decisions WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, AdminDecision>("SELECT * FROM admin_decisions ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}
