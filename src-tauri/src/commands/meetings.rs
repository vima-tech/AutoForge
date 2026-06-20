//! 会议录音处理：把会议录音的**转写文本**提炼成纪要、并拆解出多条可入流水线的需求。
//!
//! 录音→转写在前端完成（长音频分片，逐段走 `transcribe_recording_segment` → 阿里百炼
//! DashScope 实时 WS，与实时麦克风同一条链路），本模块只接收**已转写的文本**，做两件事：
//!   1. `analyze_meeting`：调会议纪要官（meeting）Agent 产出 `{ summary_md, requirements[] }`；
//!   2. `save_meeting_doc`：把纪要写入项目 `.autoforge/docs/meetings/` 工作区文档。
//!
//! 需求条目本身不在这里入池——前端复盘面板让人勾选/编辑后，逐条复用 `submit_issue`（triage）
//! 入池，保持「先审后入」（Human-Lite）与既有入池单一入口。
//!
//! 解耦铁律：业务逻辑（[`analyze_meeting_transcript`]）为纯 async fn，不依赖 Tauri 类型；
//! 命令仅做薄包装。转写文本是**不可信外部输入**，入模型前过 `has_obvious_injection`。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::Db;
use crate::models::agent::Agent;
use crate::state::AppState;

/// 转写文本上限：约 200K 字符（≈数小时会议），防止超大 IPC / 模型超长。
const MAX_TRANSCRIPT_CHARS: usize = 200_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MeetingIssue {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MeetingAnalysis {
    #[serde(default)]
    pub summary_md: String,
    #[serde(default, alias = "requirements")]
    pub issues: Vec<MeetingIssue>,
}

/// 解析会议转写：产出纪要 + 需求列表（纯逻辑，无 Tauri 依赖）。
pub async fn analyze_meeting_transcript(db: &Db, transcript: &str) -> Result<MeetingAnalysis, String> {
    let transcript = transcript.trim();
    if transcript.is_empty() {
        return Err("会议转写为空，无法分析".to_string());
    }
    let agent = resolve_meeting_agent(db).await?;

    let prompt = format!(
        "以下是一段会议录音的自动转写文本，请据此产出会议纪要与需求拆解 JSON：\n\n---\n{}\n---",
        transcript
    );
    let raw = crate::agents::llm::run_agent_text(
        db,
        &agent,
        &prompt,
        Some(crate::agents::roles::PROMPT_MEETING),
        &[],
    )
    .await
    .map_err(|e| format!("会议分析失败：{}", e))?;

    let parsed: MeetingAnalysis = crate::agents::schema::extract_json(&raw)
        .and_then(|j| serde_json::from_str(j).ok())
        .ok_or_else(|| "会议分析返回的内容无法解析为结构化结果，请重试".to_string())?;

    // 清洗：丢弃空标题条目，归一化分类/严重度到合法枚举。
    let issues = parsed
        .issues
        .into_iter()
        .filter(|r| !r.title.trim().is_empty())
        .map(|mut r| {
            r.category = normalize_category(&r.category);
            r.severity = normalize_severity(&r.severity);
            r
        })
        .collect();

    Ok(MeetingAnalysis {
        summary_md: parsed.summary_md.trim().to_string(),
        issues,
    })
}

/// 选一个可驱动会议分析的 Agent：优先 meeting / triage / analysis 角色持有者，
/// 否则任意启用且绑定了 LLM 的 Agent。会议分析量大，避免落到 claude CLI（贵且慢），
/// 故只接受绑定了 LLM 的 Agent。
async fn resolve_meeting_agent(db: &Db) -> Result<Agent, String> {
    let preferred = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE enabled=1 AND llm_id IS NOT NULL AND TRIM(COALESCE(llm_id,'')) <> ''
           AND ( (',' || COALESCE(system_kind,'') || ',') LIKE '%,meeting,%'
              OR (',' || COALESCE(system_kind,'') || ',') LIKE '%,triage,%'
              OR (',' || COALESCE(forge_role,'')  || ',') LIKE '%,analysis,%' )
         ORDER BY
           CASE WHEN (',' || COALESCE(system_kind,'') || ',') LIKE '%,meeting,%' THEN 0
                WHEN (',' || COALESCE(system_kind,'') || ',') LIKE '%,triage,%'  THEN 1
                ELSE 2 END,
           created_at
         LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(a) = preferred {
        return Ok(a);
    }
    sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents
         WHERE enabled=1 AND llm_id IS NOT NULL AND TRIM(COALESCE(llm_id,'')) <> ''
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| {
        "没有可用的已绑定 LLM 的 Agent；请在「设置 → 角色」为会议纪要官/需求分析等角色绑定一个 LLM".to_string()
    })
}

fn normalize_category(c: &str) -> String {
    match c.trim().to_ascii_lowercase().as_str() {
        "bug" => "Bug",
        "improvement" => "Improvement",
        "debt" => "Debt",
        "feature" => "Feature",
        _ => "Feature",
    }
    .to_string()
}

fn normalize_severity(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "low" => "low",
        _ => "medium",
    }
    .to_string()
}

/// 由会议标题生成文件名安全的 slug：保留字母数字与 CJK，其余折叠为 `-`，截断到 40 字。
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in title.trim().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
        if out.chars().count() >= 40 {
            break;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "meeting".to_string()
    } else {
        s
    }
}

// ── 命令（薄包装）────────────────────────────────────────────────────────────

/// 分析会议转写文本，返回纪要 + 需求列表（不落盘、不入池）。
#[tauri::command]
pub async fn analyze_meeting(
    transcript: String,
    state: State<'_, AppState>,
) -> Result<MeetingAnalysis, String> {
    if transcript.chars().count() > MAX_TRANSCRIPT_CHARS {
        return Err("会议转写过长（超过约 20 万字），请缩短录音或分段处理".to_string());
    }
    if crate::core::security::has_obvious_injection(&transcript) {
        return Err("会议转写包含可疑内容，已拒绝分析".to_string());
    }
    analyze_meeting_transcript(&state.db, &transcript).await
}

/// 把会议纪要写入项目 `.autoforge/docs/meetings/<日期>-<slug>.md`，返回相对路径。
#[tauri::command]
pub async fn save_meeting_doc(
    project_id: String,
    title: String,
    summary_md: String,
    transcript: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT repo_path FROM projects WHERE id=?")
        .bind(&project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let repo_path = row.ok_or_else(|| "项目不存在".to_string())?.0;

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let title = title.trim();
    let title = if title.is_empty() { "会议纪要" } else { title };
    let rel = format!("docs/meetings/{}-{}.md", date, slugify(title));

    let body = render_meeting_doc(title, &date, summary_md.trim(), transcript.trim());
    crate::commands::workspace::write_workspace_path(&repo_path, &rel, &body).await?;
    Ok(format!(".autoforge/{}", rel))
}

/// 渲染纪要 Markdown：标题 + 纪要正文 + 折叠的完整转写（便于回溯，又不喧宾夺主）。
fn render_meeting_doc(title: &str, date: &str, summary_md: &str, transcript: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", title));
    s.push_str(&format!("> 会议日期：{} · 由会议录音自动转写并提炼\n\n", date));
    s.push_str(summary_md);
    if !transcript.is_empty() {
        s.push_str("\n\n---\n\n<details>\n<summary>完整录音转写（自动生成，可能含识别误差）</summary>\n\n");
        s.push_str(transcript);
        s.push_str("\n\n</details>\n");
    }
    s.push('\n');
    s
}
