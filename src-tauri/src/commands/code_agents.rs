//! 代码 Agent 配置的 Tauri 命令（薄包装：取 state → 调 sqlx/纯逻辑 → 返回）。
//! 业务侧的可插拔抽象在 `agents::code_agent`；这里只做 CRUD + 默认/项目绑定 + 健康探测。
use crate::agents::code_agent::{CliCodeAgent, CliProfile, CodeAgent};
use crate::models::code_agent::{CodeAgentRow, UpsertCodeAgent};
use crate::models::code_agent_run::{CodeAgentRunLog, CodeAgentRunMeta};
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_code_agents(state: State<'_, AppState>) -> Result<Vec<CodeAgentRow>, String> {
    sqlx::query_as::<_, CodeAgentRow>("SELECT * FROM code_agents ORDER BY is_default DESC, created_at ASC")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upsert_code_agent(
    payload: UpsertCodeAgent,
    state: State<'_, AppState>,
) -> Result<CodeAgentRow, String> {
    let extra_json = serde_json::to_string(&payload.extra_args).unwrap_or_else(|_| "[]".into());
    let model = payload.model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let fast_model = payload.fast_model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let strong_model = payload.strong_model.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let id = match payload.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            // 不允许禁用当前默认 agent——否则 resolve 会落到硬兜底 claude，
            // 而 UI 默认选择器会指向一个不在候选里的死项。要先切换默认再禁用。
            if !payload.enabled {
                let is_default = sqlx::query_scalar::<_, bool>(
                    "SELECT is_default FROM code_agents WHERE id=?",
                )
                .bind(id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| e.to_string())?
                .unwrap_or(false);
                if is_default {
                    return Err("不能禁用当前默认代码 Agent，请先切换默认".into());
                }
            }
            // 更新现有：保留 is_default（由 set_default_code_agent 单独管理）。
            sqlx::query(
                "UPDATE code_agents SET kind=?, label=?, program=?, model=?, fast_model=?, strong_model=?, extra_args_json=?, enabled=? WHERE id=?",
            )
            .bind(&payload.kind)
            .bind(&payload.label)
            .bind(&payload.program)
            .bind(model)
            .bind(fast_model)
            .bind(strong_model)
            .bind(&extra_json)
            .bind(payload.enabled)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            id.to_string()
        }
        None => {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO code_agents (id, kind, label, program, model, fast_model, strong_model, extra_args_json, enabled, is_default)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
            )
            .bind(&id)
            .bind(&payload.kind)
            .bind(&payload.label)
            .bind(&payload.program)
            .bind(model)
            .bind(fast_model)
            .bind(strong_model)
            .bind(&extra_json)
            .bind(payload.enabled)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            id
        }
    };

    sqlx::query_as::<_, CodeAgentRow>("SELECT * FROM code_agents WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_code_agent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    // 不允许删掉最后一个默认；删默认前要求先改默认。
    let row = sqlx::query_as::<_, CodeAgentRow>("SELECT * FROM code_agents WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(r) = row {
        if r.is_default {
            return Err("不能删除当前默认代码 Agent，请先切换默认".into());
        }
    }
    // 清掉引用该 agent 的项目覆盖，回落全局默认。
    sqlx::query("UPDATE projects SET code_agent_id=NULL WHERE code_agent_id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM code_agents WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_default_code_agent(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM code_agents WHERE id=? AND enabled=1")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err("目标代码 Agent 不存在或未启用".into());
    }
    sqlx::query("UPDATE code_agents SET is_default=0 WHERE is_default=1")
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE code_agents SET is_default=1 WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置/清除项目级覆盖（code_agent_id=None → 跟随全局默认）。
#[tauri::command]
pub async fn set_project_code_agent(
    project_id: String,
    code_agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let val = code_agent_id.as_deref().filter(|s| !s.is_empty());
    sqlx::query("UPDATE projects SET code_agent_id=?, updated_at=datetime('now') WHERE id=?")
        .bind(val)
        .bind(&project_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// `check_code_agent_auth` 的结果：工具可用性 + （可选）当前配置模型的探测结论。
#[derive(serde::Serialize)]
pub struct CodeAgentProbe {
    /// CLI 已安装并（可探测时）登录。
    pub tool: bool,
    /// 配置模型的探测结论：None = 未探测或未配置模型；Some(true/false) = 模型可用/不可用。
    pub model: Option<bool>,
    /// 被探测的模型名（未配置/未探测时为空）。
    pub model_name: String,
    /// 失败原因或说明的简短文本（成功时为空）。
    pub detail: String,
}

/// 探测指定 code agent 是否已安装并（可探测时）登录；
/// `probe_model=true` 时额外用配置的模型发一个极小 prompt，验证模型本身可用
/// （捕捉模型名写错 / provider 未授权 / 额度耗尽等仅靠工具探测发现不了的问题）。
/// 进页面自动检测传 false（轻量、不烧 token）；点「检测可用性」按钮传 true。
#[tauri::command]
pub async fn check_code_agent_auth(
    id: String,
    probe_model: Option<bool>,
    state: State<'_, AppState>,
) -> Result<CodeAgentProbe, String> {
    let row = sqlx::query_as::<_, CodeAgentRow>("SELECT * FROM code_agents WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "代码 Agent 不存在".to_string())?;
    // 「当前配置的模型」= 主模型字段（快/强模型为按风险派生的可选项，留待执行时各自回落）。
    let model_cfg = row
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let agent = CliCodeAgent::new(CliProfile {
        kind: row.kind,
        program: row.program,
        model: row.model,
        extra_args: crate::agents::code_agent::parse_extra_args(&row.extra_args_json),
    });
    let tool = agent.check_auth().await;
    let mut probe = CodeAgentProbe {
        tool,
        model: None,
        model_name: String::new(),
        detail: String::new(),
    };
    if probe_model.unwrap_or(false) {
        match model_cfg {
            Some(m) if tool => {
                let (ok, detail) =
                    agent.probe_model(&m, std::time::Duration::from_secs(60)).await;
                probe.model = Some(ok);
                probe.model_name = m;
                probe.detail = detail;
            }
            Some(m) => {
                // 工具都没就绪，模型无从谈起。
                probe.model_name = m;
                probe.detail = "工具未就绪，跳过模型探测".into();
            }
            None => {
                probe.detail = "未配置模型（执行时用 CLI 默认）".into();
            }
        }
    }
    Ok(probe)
}

/// 列出某个 CR 的代码 Agent 执行日志（轻量元信息，不含 stdout/stderr 正文）。
/// 最新在前；上限 50 条足够覆盖一个 CR 的多次执行/重试。
#[tauri::command]
pub async fn list_code_agent_runs(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<CodeAgentRunMeta>, String> {
    sqlx::query_as::<_, CodeAgentRunMeta>(
        "SELECT id, change_request_id, worktree_session_id, phase, kind, model, exit_code,
                duration_ms, stdout_bytes, stderr_bytes, truncated, created_at
         FROM code_agent_run_logs WHERE change_request_id=?
         ORDER BY created_at DESC, rowid DESC LIMIT 50",
    )
    .bind(&cr_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 运行中编码 Agent 的实时日志快照（自任务开始累计的全文 + 下一个 chunk 序号）。
#[derive(serde::Serialize)]
pub struct RunningCodeAgentLog {
    pub text: String,
    pub next_seq: u64,
}

/// 取某 CR 运行中编码 Agent 的实时日志快照。前端中途进入「执行日志」时用它回灌已错过的开头
/// （realtime 事件只能拿到订阅之后的增量），再据 `next_seq` 与增量事件去重无缝续接。
/// 任务未运行/已结束时返回空（完整日志改由 `list_code_agent_runs` 落库列表呈现）。
#[tauri::command]
pub fn get_running_code_agent_log(cr_id: String) -> RunningCodeAgentLog {
    let (text, next_seq) = crate::state::running_log_snapshot(&cr_id);
    RunningCodeAgentLog { text, next_seq }
}

/// 取单条执行日志的完整正文（stdout/stderr），用于详情查看。
#[tauri::command]
pub async fn get_code_agent_run(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<CodeAgentRunLog>, String> {
    sqlx::query_as::<_, CodeAgentRunLog>("SELECT * FROM code_agent_run_logs WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}
