use crate::commands::attachments_common::{
    attachment_path_from_rel, sanitize_file_name, validate_attachment, MAX_ATTACHMENT_BYTES,
};
use crate::intake::{gateway, IntakeMode, IntakePayload};
use crate::models::issue::{CreateIssue, Issue, IssueAnalysis};
use crate::models::issue_attachment::{IssueAttachment, IssueAttachmentUpload};
use crate::models::job::JobPayload;
use crate::state::AppState;
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub async fn list_issues(
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    if let Some(pid) = project_id {
        sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE project_id=? ORDER BY created_at DESC",
        )
        .bind(&pid)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_as::<_, Issue>("SELECT * FROM issues ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_issue(id: String, state: State<'_, AppState>) -> Result<Option<Issue>, String> {
    sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 一页需求 + 该筛选条件下的总数。前端「全量需求总账」滚动动态加载用，
/// 避免需求量增长后一次性把全部需求拉进内存/渲染。
#[derive(serde::Serialize)]
pub struct IssuePage {
    pub items: Vec<Issue>,
    pub total: i64,
}

/// 分页查询需求的过滤参数聚合体，收拢 list_issues_page 的多个筛选字段，
/// 使 Tauri command 签名保持在 clippy::too_many_arguments 上限（7）内。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIssuesQuery {
    pub project_id: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub exclude_merged: Option<bool>,
    pub limit: i64,
    pub offset: i64,
    pub sort_asc: Option<bool>,
}

/// 分页查询需求：按项目 + 状态 + 关键字（标题/编号）过滤，按 updated_at 倒序。
/// status 为空或 "all" 表示不过滤状态；search 为空表示不过滤关键字。
/// exclude_merged 为 true 时排除「已合并」需求（与功能审计页「显示已合并需求」开关共享，默认隐藏）；
/// 当显式按 merged 状态筛选时该排除自然失效（status 优先）。
#[tauri::command]
pub async fn list_issues_page(
    query: ListIssuesQuery,
    state: State<'_, AppState>,
) -> Result<IssuePage, String> {
    use sqlx::{QueryBuilder, Sqlite};
    let ListIssuesQuery { project_id, status, search, exclude_merged, limit, offset, sort_asc } = query;
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let status = status.filter(|s| !s.is_empty() && s != "all");
    // 仅当未显式筛选 merged 时才生效（显式选 merged 说明用户主动想看，不再隐藏）。
    let exclude_merged =
        exclude_merged.unwrap_or(false) && status.as_deref() != Some("merged");
    // LIKE 通配串作为局部变量，确保在两次 query 构建期间存活。
    let like = search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{}%", s));

    let total: i64 = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        qb.build_query_scalar()
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let items = {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE 1=1");
        if let Some(pid) = &project_id {
            qb.push(" AND project_id = ").push_bind(pid);
        }
        if let Some(st) = &status {
            qb.push(" AND status = ").push_bind(st);
        }
        if exclude_merged {
            qb.push(" AND status != 'merged'");
        }
        if let Some(l) = &like {
            qb.push(" AND (title LIKE ")
                .push_bind(l)
                .push(" OR id LIKE ")
                .push_bind(l)
                .push(")");
        }
        // 统一按创建时间排序（updated_at 会随执行/重分析变动，跨页不稳定）。
        // 默认正序：旧需求置前，避免问题积压长期无人处理。
        qb.push(if sort_asc.unwrap_or(true) {
            " ORDER BY created_at ASC LIMIT "
        } else {
            " ORDER BY created_at DESC LIMIT "
        })
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
        qb.build_query_as::<Issue>()
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(IssuePage { items, total })
}

/// 某项目下出现过的所有状态（去重），用于总账的状态筛选 chip——
/// 不依赖已加载的那一页，仍能列出全部可筛选状态。
#[tauri::command]
pub async fn list_issue_statuses(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT DISTINCT status FROM issues WHERE project_id=?")
        .bind(&project_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 按状态集合取需求（完整字段），用于「需求审核」队列等有界子集，
/// 替代为拿少量在产需求而全量加载 issues。
#[tauri::command]
pub async fn list_issues_by_statuses(
    project_id: String,
    statuses: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Issue>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if statuses.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM issues WHERE project_id = ");
    qb.push_bind(&project_id).push(" AND status IN (");
    let mut sep = qb.separated(", ");
    for s in &statuses {
        sep.push_bind(s);
    }
    qb.push(") ORDER BY created_at DESC");
    qb.build_query_as::<Issue>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// 跨项目「待审核需求」计数：按 project_id 分组统计 pending_issue_review 数量。
/// 用于功能审计页项目列表的需求待审徽标——只取计数，避免全量加载 issues 行。
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct ProjectIssueReviewCount {
    pub project_id: String,
    pub count: i64,
}

#[tauri::command]
pub async fn count_pending_issue_reviews(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectIssueReviewCount>, String> {
    sqlx::query_as::<_, ProjectIssueReviewCount>(
        "SELECT project_id, COUNT(*) AS count FROM issues \
         WHERE status = 'pending_issue_review' GROUP BY project_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

// ── 总账导出（全量 / 按状态类型多选）────────────────────────────────────────
// 「全量需求总账」页的导出：可全量导出，或多选状态类型只导其中几类；
// 支持 CSV / Excel(xlsx)，xlsx 可「按类型分表」（每个状态一个工作表）。
// 写入系统下载目录并在文件管理器中定位，沿用 export_bulk_template 的落盘模式。

/// 状态码 → 中文标签（与前端 LEDGER_STATUS_LABEL 对齐；缺省回落原始状态码）。
fn issue_status_label(status: &str) -> &str {
    match status {
        "triage" => "待整理",
        "pending_analysis" => "分析中",
        "analysis_failed" => "分析失败",
        "pending_issue_review" => "需求审核",
        "pending_execution" => "待编码",
        "executing" => "编码中",
        "pending_code_review" => "代码审核",
        "pending_merge" => "待合并",
        "merge_testing" => "合并中",
        "merge_ready" => "待落地",
        "merged" => "已合并",
        "reverting" => "撤销中",
        "reverted" => "已撤销",
        "rejected" => "已拒绝",
        "deferred" => "暂不处置",
        "merge_failed" => "合并失败",
        "merge_conflict" => "合并冲突",
        "execution_failed" => "执行失败",
        "no_change_needed" => "无需改动",
        other => other,
    }
}

/// 导出列头（与导出行字段顺序一致）。
const EXPORT_HEADERS: [&str; 8] = [
    "编号", "标题", "状态", "分类", "严重度", "来源", "创建时间", "更新时间",
];

/// 取一条需求的导出行（与 EXPORT_HEADERS 顺序对应）。
fn issue_export_row(i: &Issue) -> [String; 8] {
    [
        i.id.clone(),
        i.title.clone(),
        issue_status_label(&i.status).to_string(),
        i.category.clone(),
        i.severity.clone(),
        i.source_type.clone(),
        i.created_at.clone(),
        i.updated_at.clone(),
    ]
}

/// CSV 单元格转义：含逗号/引号/换行的值用双引号包裹并转义内部引号。
fn csv_escape(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// 构建 CSV 字节（带 UTF-8 BOM，Excel 打开不乱码）。
fn issues_export_csv(issues: &[Issue]) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&EXPORT_HEADERS.join(","));
    s.push('\n');
    for i in issues {
        let row = issue_export_row(i);
        let line: Vec<String> = row.iter().map(|c| csv_escape(c)).collect();
        s.push_str(&line.join(","));
        s.push('\n');
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(s.as_bytes());
    bytes
}

/// 构建 xlsx 字节。split_by_type=true 时按状态分表（每个状态一个工作表），
/// 否则全部写入单个「需求」工作表（含状态列）。
fn issues_export_xlsx(issues: &[Issue], split_by_type: bool) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut workbook = Workbook::new();
    let header_fmt = Format::new().set_bold().set_background_color(0xE8772E);

    // 把若干行写进一个工作表（含表头）。
    let write_sheet = |workbook: &mut Workbook, name: &str, rows: &[&Issue]| -> Result<(), String> {
        // 工作表名 ≤31 字符且禁含 []:*?/\\，做一次净化。
        let mut safe: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).collect();
        if safe.chars().count() > 31 {
            safe = safe.chars().take(31).collect();
        }
        if safe.is_empty() {
            safe = "需求".to_string();
        }
        let sheet = workbook
            .add_worksheet()
            .set_name(&safe)
            .map_err(|e| e.to_string())?;
        for (col, h) in EXPORT_HEADERS.iter().enumerate() {
            sheet
                .write_string_with_format(0, col as u16, *h, &header_fmt)
                .map_err(|e| e.to_string())?;
        }
        for (r, issue) in rows.iter().enumerate() {
            let row = issue_export_row(issue);
            for (col, v) in row.iter().enumerate() {
                sheet
                    .write_string(r as u32 + 1, col as u16, v)
                    .map_err(|e| e.to_string())?;
            }
        }
        sheet.set_column_width(0, 22).ok();
        sheet.set_column_width(1, 40).ok();
        sheet.set_column_width(2, 12).ok();
        sheet.set_column_width(6, 20).ok();
        sheet.set_column_width(7, 20).ok();
        Ok(())
    };

    if split_by_type {
        // 按状态分组，保持稳定顺序（首次出现的先后）。
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<&Issue>> =
            std::collections::HashMap::new();
        for i in issues {
            if !groups.contains_key(&i.status) {
                order.push(i.status.clone());
            }
            groups.entry(i.status.clone()).or_default().push(i);
        }
        if order.is_empty() {
            // 无数据也产一张空表，避免无工作表导致 xlsx 非法。
            write_sheet(&mut workbook, "需求", &[])?;
        }
        for st in &order {
            let rows = groups.get(st).map(|v| v.as_slice()).unwrap_or(&[]);
            write_sheet(&mut workbook, issue_status_label(st), rows)?;
        }
    } else {
        let rows: Vec<&Issue> = issues.iter().collect();
        write_sheet(&mut workbook, "需求", &rows)?;
    }

    workbook.save_to_buffer().map_err(|e| e.to_string())
}

/// 导出参数：项目 + 状态类型多选（空=全量）+ 格式 + 是否按类型分表。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportIssuesQuery {
    pub project_id: String,
    /// 选中的状态类型；为空表示全量导出（不按状态过滤）。
    #[serde(default)]
    pub statuses: Vec<String>,
    /// "csv" | "xlsx"
    pub format: String,
    /// 仅 xlsx 生效：每个状态类型导出为独立工作表。
    #[serde(default)]
    pub split_by_type: bool,
}

/// 导出结果：落盘路径 + 导出条数。
#[derive(serde::Serialize)]
pub struct IssueExportResult {
    pub path: String,
    pub count: i64,
}

/// 导出总账需求到系统下载目录（CSV / xlsx），返回文件路径与条数。
#[tauri::command]
pub async fn export_issues(
    query: ExportIssuesQuery,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<IssueExportResult, String> {
    use sqlx::{QueryBuilder, Sqlite};
    use tauri::Manager;
    use tauri_plugin_opener::OpenerExt;

    let ExportIssuesQuery { project_id, statuses, format, split_by_type } = query;
    let fmt = format.to_lowercase();
    if fmt != "csv" && fmt != "xlsx" {
        return Err(format!("不支持的导出格式: {}，请使用 csv / xlsx", format));
    }

    // 拉取需求（按创建时间正序，与总账一致的稳定排序）。
    let issues: Vec<Issue> = {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT * FROM issues WHERE project_id = ");
        qb.push_bind(&project_id);
        if !statuses.is_empty() {
            qb.push(" AND status IN (");
            let mut sep = qb.separated(", ");
            for s in &statuses {
                sep.push_bind(s);
            }
            qb.push(")");
        }
        qb.push(" ORDER BY created_at ASC");
        qb.build_query_as::<Issue>()
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let count = issues.len() as i64;

    // 文件名：项目无关，带类型与时间戳避免覆盖。
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let (file_name, bytes) = if fmt == "csv" {
        (
            format!("autoforge-需求总账-{}.csv", stamp),
            issues_export_csv(&issues),
        )
    } else {
        (
            format!("autoforge-需求总账-{}.xlsx", stamp),
            issues_export_xlsx(&issues, split_by_type)?,
        )
    };

    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir());
    let dest = dir.join(&file_name);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("写入导出文件失败: {}", e))?;

    let dest_str = dest.to_string_lossy().to_string();
    let _ = app.opener().reveal_item_in_dir(&dest_str);

    Ok(IssueExportResult { path: dest_str, count })
}

/// 需求标题（轻量），按 id 批量取，用于变更请求列表解析标题，
/// 无需把全部需求载入内存。
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct IssueTitle {
    pub id: String,
    pub title: String,
}

#[tauri::command]
pub async fn list_issue_titles(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<IssueTitle>, String> {
    use sqlx::{QueryBuilder, Sqlite};
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT id, title FROM issues WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id);
    }
    qb.push(")");
    qb.build_query_as::<IssueTitle>()
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_issue_analysis(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Option<IssueAnalysis>, String> {
    sqlx::query_as::<_, IssueAnalysis>("SELECT * FROM issue_analyses WHERE issue_id=?")
        .bind(&issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn submit_issue(
    payload: CreateIssue,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Issue, String> {
    let mode = IntakeMode::from_opt(payload.mode.as_deref());
    let has_bug = payload.repro_steps.is_some()
        || payload.environment.is_some()
        || payload.expected.is_some()
        || payload.actual.is_some();
    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: payload.source_type.unwrap_or_else(|| "manual".to_string()),
        source_ref: payload.source_ref,
    };
    let issue = gateway::receive(&state.db, &state.job_tx, &app, intake, mode).await?;

    // Bug 载体字段单独落库（保持 IntakePayload 与六通道不变）。
    if has_bug {
        sqlx::query(
            "UPDATE issues SET repro_steps=?, environment=?, expected=?, actual=? WHERE id=?",
        )
        .bind(&payload.repro_steps)
        .bind(&payload.environment)
        .bind(&payload.expected)
        .bind(&payload.actual)
        .bind(&issue.id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        return sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id=?")
            .bind(&issue.id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string());
    }
    Ok(issue)
}

/// Re-run requirement analysis for an issue whose analysis failed (or is stuck at
/// review 1). Resets the issue to `pending_analysis` and re-enqueues the analysis
/// job under a UNIQUE idempotency key (`analysis:<id>:retry:<uuid>`). The stable
/// `analysis:<id>` key would silently de-duplicate against an already-`completed`
/// job row — `enqueue` only re-dispatches a `failed` row — leaving the issue parked
/// at `pending_analysis` forever. A unique key always dispatches, so a transient LLM
/// failure (or a re-analysis of an already-analyzed issue) recovers in one click.
/// The status-guarded UPDATE above already idempotently blocks a double-click.
#[tauri::command]
pub async fn retry_analysis(
    issue_id: String,
    _app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let updated = sqlx::query(
        "UPDATE issues SET status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('analysis_failed', 'pending_issue_review')",
    )
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「分析失败」或「待需求审核」状态的需求可重新分析".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 需求审核 补充意见重评：管理员在「待需求审核」阶段提交补充意见，需求带着该意见
/// 重新分析，再回到需求审核。意见落库到 issues.review_feedback（一次性，被分析任务
/// 消费后清空）。意见为人工输入，入库前过 has_obvious_injection 防注入。
#[tauri::command]
pub async fn reanalyze_with_feedback(
    issue_id: String,
    feedback: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let feedback = feedback.trim().to_string();
    if feedback.is_empty() {
        return Err("补充意见不能为空".to_string());
    }
    if crate::core::security::has_obvious_injection(&feedback) {
        return Err("补充意见包含可疑指令，已拦截".to_string());
    }
    // 仅「待需求审核」或「分析失败」状态可带意见重评，避免影响在产/已完结需求。
    let updated = sqlx::query(
        "UPDATE issues SET review_feedback=?, status='pending_analysis', updated_at=datetime('now')
         WHERE id=? AND status IN ('pending_issue_review', 'analysis_failed')",
    )
    .bind(&feedback)
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「待需求审核」或「分析失败」状态的需求可提交补充意见重新评估".to_string());
    }
    crate::tasks::runner::enqueue(
        &state.db,
        &state.job_tx,
        "analysis",
        // 唯一 key：稳定的 analysis:<id> 会被 enqueue 按已 completed 的旧 job 行去重而
        // 不再派发，需求会永远停在 pending_analysis。带意见重评必须真正重跑。
        &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
        JobPayload::Analysis {
            issue_id: issue_id.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 「暂不处置」：把待审核 / 分析失败的需求搁置为 `deferred`。
///
/// 搁置态是临时停泊，不进入流水线、不能直接编码——这类需求很可能因项目演进发生变化，
/// 重新启用时只能走「重新分析」（见 `reactivate_issue`）。仅允许从「待需求审核」「分析失败」
/// 「分析中」三种尚未落地的需求侧状态进入，避免与运行中/已落地的下游产物脱节。
#[tauri::command]
pub async fn defer_issue(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let updated = sqlx::query(
        "UPDATE issues SET status='deferred', updated_at=datetime('now')
         WHERE id=? AND status IN ('pending_issue_review', 'analysis_failed', 'pending_analysis')",
    )
    .bind(&issue_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("仅「待需求审核」「分析失败」「分析中」状态的需求可暂不处置".to_string());
    }
    Ok(())
}

/// 重新启用被「拒绝」或「暂不处置」的需求。
///
/// - `mode="reanalyze"`：从 `rejected` / `deferred` 回到 `pending_analysis` 并重新入队分析。
///   搁置需求**只能**走这条路——项目可能已演进，必须重判而非直接复用旧结论。
/// - `mode="review"`：仅 `rejected` 可用，且必须已有分析结果，直接退回 `pending_issue_review`
///   省去重跑分析。搁置需求显式拒绝此模式。
///
/// 重新分析复用 `analysis:<id>:retry:<uuid>` 唯一幂等键，规避稳定键被旧 completed job 去重。
#[tauri::command]
pub async fn reactivate_issue(
    issue_id: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM issues WHERE id=?")
        .bind(&issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let Some(status) = status else {
        return Err("需求不存在".to_string());
    };
    if status != "rejected" && status != "deferred" {
        return Err("仅「已拒绝」或「暂不处置」状态的需求可重新启用".to_string());
    }

    match mode.as_str() {
        "reanalyze" => {
            sqlx::query(
                "UPDATE issues SET status='pending_analysis', updated_at=datetime('now') WHERE id=?",
            )
            .bind(&issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            crate::tasks::runner::enqueue(
                &state.db,
                &state.job_tx,
                "analysis",
                &format!("analysis:{}:retry:{}", issue_id, uuid::Uuid::new_v4()),
                JobPayload::Analysis {
                    issue_id: issue_id.clone(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        "review" => {
            if status != "rejected" {
                return Err("「暂不处置」的需求只能重新分析，不能直接退回需求审核".to_string());
            }
            let has_analysis: Option<(String,)> =
                sqlx::query_as("SELECT id FROM issue_analyses WHERE issue_id=? LIMIT 1")
                    .bind(&issue_id)
                    .fetch_optional(&state.db)
                    .await
                    .map_err(|e| e.to_string())?;
            if has_analysis.is_none() {
                return Err("该需求尚无分析结果，请改用「重新分析」".to_string());
            }
            sqlx::query(
                "UPDATE issues SET status='pending_issue_review', updated_at=datetime('now') WHERE id=?",
            )
            .bind(&issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        _ => Err("未知的重新启用模式".to_string()),
    }
}

/// 列出某 CR 的测试遥测记录（review_2 合并前自动测试结果）。
#[tauri::command]
pub async fn list_cr_test_runs(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::test_run::CrTestRun>, String> {
    sqlx::query_as::<_, crate::models::test_run::CrTestRun>(
        "SELECT * FROM cr_test_runs WHERE cr_id=? ORDER BY run_at DESC",
    )
    .bind(&cr_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 人审改 AI 生成的验收标准（acceptance_json，JSON 数组字符串）。
#[tauri::command]
pub async fn update_issue_acceptance(
    issue_id: String,
    acceptance_json: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 校验是合法 JSON，避免写入脏数据破坏后续解析。
    serde_json::from_str::<serde_json::Value>(&acceptance_json)
        .map_err(|e| format!("验收标准 JSON 非法：{}", e))?;
    sqlx::query("UPDATE issues SET acceptance_json=?, updated_at=datetime('now') WHERE id=?")
        .bind(&acceptance_json)
        .bind(&issue_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 需求附件（issue_attachments）──────────────────────────────────────────────
// 镜像会议室附件（conversations.rs::import_attachment），存储基目录用
// attachments_base()/issues/<issue_id>/，复用 attachments_common 的安全白名单。
// 图片附件可经 supports_vision 的分析 Agent 内联识别（tasks/analysis.rs）。

async fn load_issue_attachment(
    db: &crate::db::Db,
    attachment_id: &str,
) -> Result<IssueAttachment, String> {
    sqlx::query_as::<_, IssueAttachment>("SELECT * FROM issue_attachments WHERE id=?")
        .bind(attachment_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "附件不存在".to_string())
}

#[tauri::command]
pub async fn import_issue_attachment(
    payload: IssueAttachmentUpload,
    state: State<'_, AppState>,
) -> Result<IssueAttachment, String> {
    let issue_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM issues WHERE id=?")
        .bind(&payload.issue_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if issue_exists.is_none() {
        return Err(format!("issue {} not found", payload.issue_id));
    }

    if payload.data_base64.len() > (MAX_ATTACHMENT_BYTES * 4 / 3 + 16) {
        return Err("附件超过 10 MB 上限".to_string());
    }

    let bytes = general_purpose::STANDARD
        .decode(payload.data_base64.as_bytes())
        .map_err(|_| "附件内容不是有效的 base64".to_string())?;
    if bytes.is_empty() {
        return Err("附件不能为空".to_string());
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err("附件超过 10 MB 上限".to_string());
    }

    let clean_name = sanitize_file_name(&payload.file_name);
    let policy = validate_attachment(&clean_name, payload.mime_hint.as_str(), &bytes)?;
    let attachment_id = Uuid::new_v4().to_string();
    let stored_name = format!("{}.{}", attachment_id, policy.ext);
    // rel_path 含 `issues/` 段，使 attachment_path_from_rel（= attachments_base()/rel_path）
    // 与下方落盘目录一致。
    let rel_path = format!("issues/{}/{}", payload.issue_id, stored_name);
    let issue_dir = PathBuf::from(crate::state::attachments_base())
        .join("issues")
        .join(&payload.issue_id);
    let file_path = issue_dir.join(&stored_name);

    tokio::fs::create_dir_all(&issue_dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| e.to_string())?;

    let sha256 = hex::encode(Sha256::digest(&bytes));
    let insert_result = sqlx::query(
        "INSERT INTO issue_attachments
         (id, issue_id, original_name, stored_name, rel_path, mime, kind, size_bytes, sha256)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&attachment_id)
    .bind(&payload.issue_id)
    .bind(&clean_name)
    .bind(&stored_name)
    .bind(&rel_path)
    .bind(policy.mime)
    .bind(policy.kind)
    .bind(bytes.len() as i64)
    .bind(&sha256)
    .execute(&state.db)
    .await;

    if let Err(e) = insert_result {
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err(e.to_string());
    }

    load_issue_attachment(&state.db, &attachment_id).await
}

#[tauri::command]
pub async fn list_issue_attachments(
    issue_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<IssueAttachment>, String> {
    sqlx::query_as::<_, IssueAttachment>(
        "SELECT * FROM issue_attachments WHERE issue_id=? ORDER BY created_at ASC",
    )
    .bind(&issue_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn issue_attachment_data_url(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    if attachment.kind != "image" {
        return Err("只有图片附件支持内联预览".to_string());
    }
    if attachment.size_bytes as usize > MAX_ATTACHMENT_BYTES {
        return Err("图片超过预览大小上限".to_string());
    }
    let path = attachment_path_from_rel(&attachment.rel_path)?;
    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{};base64,{}",
        attachment.mime,
        general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub async fn open_issue_attachment(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    let path = attachment_path_from_rel(&attachment.rel_path)?;
    if !path.exists() {
        return Err("附件文件不存在或已被移除".to_string());
    }

    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    cmd.arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开附件：{}", e))
}

#[tauri::command]
pub async fn delete_issue_attachment(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let attachment = load_issue_attachment(&state.db, &attachment_id).await?;
    if let Ok(path) = attachment_path_from_rel(&attachment.rel_path) {
        let _ = tokio::fs::remove_file(&path).await;
    }
    sqlx::query("DELETE FROM issue_attachments WHERE id=?")
        .bind(&attachment_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
