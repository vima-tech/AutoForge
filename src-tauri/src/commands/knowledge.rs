//! Meeting-room Innate commands + self-growth configuration.
//!
//! Exposes the `/remember`, `/recall`, `/evolve`, `/innate` slash commands the
//! chat composer dispatches, plus get/set for the background self-growth knobs
//! (evolve interval + capture threshold) stored in `app_settings`.

use crate::core::event;
use crate::models::conversation::{Conversation, Message};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, State};
use uuid::Uuid;

/// Pseudo-sender id for system messages produced by Innate commands. The
/// frontend special-cases this to render an "Innate" labelled, ember-styled row.
pub const INNATE_SENDER: &str = "__innate__";

// ── Self-growth settings ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSettings {
    /// Hours between background evolve sweeps over active projects. `0` disables
    /// the timer (event-threshold trigger still applies).
    pub evolve_interval_hours: u32,
    /// Captures per project that trigger an automatic background evolve. `0`
    /// disables the event trigger (timer backstop still applies).
    pub capture_threshold: u32,
}

impl Default for KnowledgeSettings {
    fn default() -> Self {
        Self { evolve_interval_hours: 12, capture_threshold: 8 }
    }
}

/// Load self-growth settings (shared by the command layer and lib.rs setup).
pub async fn load_knowledge_settings(db: &crate::db::Db) -> KnowledgeSettings {
    let mut s = KnowledgeSettings::default();
    if let Ok(rows) = sqlx::query_as::<_, (String, String)>(
        "SELECT key, value FROM app_settings
         WHERE key IN ('knowledge.evolve_interval_hours', 'knowledge.capture_threshold')",
    )
    .fetch_all(db)
    .await
    {
        for (key, value) in rows {
            match key.as_str() {
                "knowledge.evolve_interval_hours" => {
                    s.evolve_interval_hours = value.parse().unwrap_or(s.evolve_interval_hours);
                }
                "knowledge.capture_threshold" => {
                    s.capture_threshold = value.parse().unwrap_or(s.capture_threshold);
                }
                _ => {}
            }
        }
    }
    s
}

#[tauri::command]
pub async fn get_knowledge_settings(
    state: State<'_, AppState>,
) -> Result<KnowledgeSettings, String> {
    Ok(load_knowledge_settings(&state.db).await)
}

#[tauri::command]
pub async fn set_knowledge_settings(
    payload: KnowledgeSettings,
    state: State<'_, AppState>,
) -> Result<KnowledgeSettings, String> {
    let interval = payload.evolve_interval_hours.min(24 * 30);
    let threshold = payload.capture_threshold.min(1000);
    for (key, value) in [
        ("knowledge.evolve_interval_hours", interval.to_string()),
        ("knowledge.capture_threshold", threshold.to_string()),
    ] {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(&value)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    // Apply the new threshold to the running process immediately.
    crate::knowledge::set_evolve_threshold(threshold);
    Ok(KnowledgeSettings { evolve_interval_hours: interval, capture_threshold: threshold })
}

// ── Embedding 模型配置 ──────────────────────────────────────────────────────
//
// Innate 的 embedding（用于 recall 语义检索）无法用聊天 LLM 表达（需 provider/
// base_url/model_id/api_key/dim），llm_configs 也无此概念，故以 `knowledge.embedding.*`
// 前缀键独立存 app_settings，保存时刷新 in-process Innate 的 embedding provider。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSettings {
    /// 目前 Innate 仅支持 OpenAI 兼容的 embedding API。
    pub provider: String,
    pub base_url: String,
    pub model_id: String,
    pub api_key: String,
    /// 向量维度，必须与模型实际产出一致（不一致会静默导致相似度错误）。
    pub dim: u32,
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            base_url: String::new(),
            model_id: String::new(),
            api_key: String::new(),
            dim: 1536,
        }
    }
}

/// Load embedding settings (shared by the command layer and the Innate sync).
pub async fn load_embedding_settings(db: &crate::db::Db) -> EmbeddingSettings {
    let mut s = EmbeddingSettings::default();
    if let Ok(rows) = sqlx::query_as::<_, (String, String)>(
        "SELECT key, value FROM app_settings WHERE key IN (
            'knowledge.embedding.provider', 'knowledge.embedding.base_url',
            'knowledge.embedding.model_id', 'knowledge.embedding.api_key',
            'knowledge.embedding.dim')",
    )
    .fetch_all(db)
    .await
    {
        for (key, value) in rows {
            match key.as_str() {
                "knowledge.embedding.provider" if !value.is_empty() => s.provider = value,
                "knowledge.embedding.base_url" => s.base_url = value,
                "knowledge.embedding.model_id" => s.model_id = value,
                "knowledge.embedding.api_key" => s.api_key = value,
                "knowledge.embedding.dim" => s.dim = value.parse().unwrap_or(s.dim),
                _ => {}
            }
        }
    }
    s
}

#[tauri::command]
pub async fn get_knowledge_embedding(
    state: State<'_, AppState>,
) -> Result<EmbeddingSettings, String> {
    Ok(load_embedding_settings(&state.db).await)
}

#[tauri::command]
pub async fn set_knowledge_embedding(
    payload: EmbeddingSettings,
    state: State<'_, AppState>,
) -> Result<EmbeddingSettings, String> {
    let dim = payload.dim.clamp(1, 8192);
    for (key, value) in [
        ("knowledge.embedding.provider", payload.provider.trim().to_string()),
        ("knowledge.embedding.base_url", payload.base_url.trim().to_string()),
        ("knowledge.embedding.model_id", payload.model_id.trim().to_string()),
        ("knowledge.embedding.api_key", payload.api_key.trim().to_string()),
        ("knowledge.embedding.dim", dim.to_string()),
    ] {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(&value)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }
    // 刷新 in-process Innate 的 embedding/蒸馏模型配置（下次 recall/evolve 即生效）。
    crate::knowledge::refresh_kb_models(&state.db).await;
    Ok(load_embedding_settings(&state.db).await)
}

// ── /command dispatch ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConvCommandPayload {
    pub conversation_id: String,
    /// Bare command name without the leading slash: remember | recall | evolve | innate.
    pub command: String,
    /// Free-text argument after the command (may be empty).
    #[serde(default)]
    pub arg: String,
}

/// Run an Innate slash command in a conversation and post the result as a
/// system message. The frontend has already persisted the user's `/…` message.
#[tauri::command]
pub async fn run_conversation_command(
    payload: ConvCommandPayload,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Message, String> {
    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&payload.conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", payload.conversation_id))?;

    let project_id = conv.project_id.as_deref();
    let scope_label = if project_id.is_some() { "本项目知识库" } else { "通用知识库（跨项目）" };
    let arg = payload.arg.trim().to_string();

    let body = match payload.command.as_str() {
        "remember" => {
            let content = if arg.is_empty() {
                recent_conversation_text(&state.db, &payload.conversation_id).await
            } else {
                arg.clone()
            };
            if content.trim().is_empty() {
                "⚠️ 没有可存储的内容。请在 `/remember` 后写明要记住的经验，或在有对话内容时再调用。".to_string()
            } else {
                let trigger = first_line(&content, 200);
                match crate::knowledge::cmd_remember(project_id, &content, &trigger).await {
                    Ok(()) => format!(
                        "🧠 **已存入{}**\n\n> {}",
                        scope_label,
                        truncate(&content, 280).replace('\n', "\n> ")
                    ),
                    Err(e) => format!("❌ 存储失败：{}", e),
                }
            }
        }
        "recall" => match crate::knowledge::cmd_recall(project_id, &arg).await {
            Ok(text) if !text.is_empty() => {
                format!("🔎 **{} · 召回结果**\n\n{}", scope_label, text)
            }
            Ok(_) => format!("🔎 {} 暂无与「{}」相关的经验。", scope_label, arg),
            Err(e) => format!("❌ 召回失败：{}", e),
        },
        "evolve" => match crate::knowledge::cmd_evolve(project_id).await {
            Ok(text) => {
                crate::knowledge::reset_capture_count(project_id);
                let detail = if text.is_empty() { "（无新增蒸馏）".to_string() } else { truncate(&text, 600) };
                format!("⚙️ **已触发{}进化（蒸馏 + 整理）**\n\n{}", scope_label, detail)
            }
            Err(e) => format!("❌ 进化失败：{}", e),
        },
        "innate" => match crate::knowledge::cmd_inspect(project_id).await {
            Ok(text) if !text.is_empty() => {
                format!("📊 **{} · 健康度**\n\n```\n{}\n```", scope_label, truncate(&text, 1500))
            }
            Ok(_) => format!("📊 {} 暂无数据。", scope_label),
            Err(e) => format!("❌ 状态查询失败：{}", e),
        },
        other => format!(
            "未知命令 `/{}`。可用命令：`/remember` 存知识 · `/recall <关键词>` 召回 · `/evolve` 立即进化 · `/innate` 状态查看",
            other
        ),
    };

    post_innate_message(&app, &state.db, &payload.conversation_id, &body).await
}

/// Insert a system message authored by Innate and emit the receive event.
async fn post_innate_message(
    app: &AppHandle,
    db: &crate::db::Db,
    conversation_id: &str,
    markdown: &str,
) -> Result<Message, String> {
    let blocks = serde_json::json!([{ "t": "md", "md": markdown }]);
    let content_json = blocks.to_string();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(conversation_id)
    .bind(INNATE_SENDER)
    .bind(&content_json)
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;

    event::emit(
        app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.to_string(),
            message_id: id.clone(),
        },
    );

    sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id=?")
        .bind(&id)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())
}

// ── helpers ────────────────────────────────────────────────────────────────

/// Concatenate readable text from the most recent messages (newest excluded if
/// it's a slash command) for a `/remember` with no explicit argument.
async fn recent_conversation_text(db: &crate::db::Db, conversation_id: &str) -> String {
    let rows = sqlx::query_as::<_, (Option<String>, String)>(
        "SELECT from_agent, content_json FROM messages
         WHERE conversation_id=? AND excluded_from_context=0
         ORDER BY created_at DESC LIMIT 8",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    for (from_agent, content_json) in rows {
        let text = message_plain_text(&content_json);
        let text = text.trim();
        if text.is_empty() || text.starts_with('/') {
            continue;
        }
        let who = match from_agent.as_deref() {
            None => "用户",
            Some(INNATE_SENDER) => continue,
            Some(_) => "Agent",
        };
        lines.push(format!("{}：{}", who, text));
    }
    lines.reverse();
    truncate(&lines.join("\n"), 3500)
}

/// Extract plain text from a message's content_json (md / code blocks).
fn message_plain_text(content_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(content_json) else {
        return String::new();
    };
    let Some(blocks) = value.as_array() else {
        return String::new();
    };
    let mut parts = Vec::new();
    for b in blocks {
        match b.get("t").and_then(|t| t.as_str()) {
            Some("md") => {
                if let Some(s) = b.get("md").and_then(|v| v.as_str()) {
                    parts.push(s.to_string());
                }
            }
            Some("code") => {
                if let Some(s) = b.get("code").and_then(|v| v.as_str()) {
                    parts.push(s.to_string());
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    truncate(line, max)
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(max).collect();
        format!("{}…", cut)
    }
}
