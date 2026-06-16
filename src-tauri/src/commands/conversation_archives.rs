use crate::models::conversation_archive::{
    ArchiveSearchHit, ArchivedMessage, ConversationArchiveDetail, ConversationArchiveSummary,
};
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

const INNATE_SENDER: &str = "__innate__";

#[derive(sqlx::FromRow)]
struct ArchiveMsgRow {
    from_agent: Option<String>,
    content_json: String,
    created_at: String,
    #[sqlx(default)]
    excluded_from_context: bool,
    agent_name: Option<String>,
    agent_en: Option<String>,
    agent_color: Option<String>,
}

/// 从消息 content_json 中提取纯文本（与前端 msgText 对齐），用于全文检索。
fn extract_plain_text(content_json: &str) -> String {
    fn push_field(out: &mut String, b: &serde_json::Value, key: &str) {
        if let Some(s) = b.get(key).and_then(|v| v.as_str()) {
            out.push_str(s);
            out.push(' ');
        }
    }
    let mut out = String::new();
    if let Ok(serde_json::Value::Array(blocks)) =
        serde_json::from_str::<serde_json::Value>(content_json)
    {
        for b in blocks {
            match b.get("t").and_then(|v| v.as_str()).unwrap_or("") {
                "md" => push_field(&mut out, &b, "md"),
                "code" => push_field(&mut out, &b, "code"),
                "file" => {
                    push_field(&mut out, &b, "name");
                    push_field(&mut out, &b, "meta");
                }
                "image" => {
                    push_field(&mut out, &b, "label");
                    push_field(&mut out, &b, "meta");
                }
                "artifact" => {
                    push_field(&mut out, &b, "kind");
                    push_field(&mut out, &b, "title");
                    push_field(&mut out, &b, "body");
                }
                "file_written" => {
                    push_field(&mut out, &b, "path");
                    push_field(&mut out, &b, "content");
                }
                _ => {}
            }
        }
    }
    out
}

/// 统计 query 在 text 中出现次数，并截取首个命中前后的片段。按字符（非字节）窗口，兼容中文。
fn count_and_snippet(text: &str, query: &str) -> (i64, String) {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return (0, String::new());
    }
    let hay = text.to_lowercase();
    let count = hay.matches(&needle).count() as i64;
    let snippet = match hay.find(&needle) {
        Some(byte_pos) => {
            let char_start = hay[..byte_pos].chars().count();
            let chars: Vec<char> = text.chars().collect();
            let needle_len = needle.chars().count();
            let from = char_start.saturating_sub(20);
            let to = (char_start + needle_len + 40).min(chars.len());
            let mut s = String::new();
            if from > 0 {
                s.push('…');
            }
            s.extend(&chars[from..to]);
            if to < chars.len() {
                s.push('…');
            }
            s.replace(['\n', '\r'], " ").trim().to_string()
        }
        None => String::new(),
    };
    (count, snippet)
}

/// 归档会议室：把当前消息打包成不可变只读快照，随后清空会议室（房间保留可继续使用）。
#[tauri::command]
pub async fn archive_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<ConversationArchiveSummary, String> {
    let conv: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT type AS conv_type, name, project_id FROM conversations WHERE id=?")
            .bind(&conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let (conv_type, name, project_id) =
        conv.ok_or_else(|| format!("conversation {} not found", conversation_id))?;

    // 标题：群聊用群名；单聊用对端 Agent 名。
    let title = match name {
        Some(n) if !n.trim().is_empty() => n,
        _ => {
            let agent_name: Option<(String,)> = sqlx::query_as(
                "SELECT a.name FROM conversation_members cm
                 JOIN agents a ON a.id = cm.agent_id
                 WHERE cm.conversation_id = ? LIMIT 1",
            )
            .bind(&conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            agent_name.map(|(n,)| n).unwrap_or_else(|| "会议室".to_string())
        }
    };

    let project_name: Option<String> = if let Some(pid) = project_id.as_deref() {
        sqlx::query_as::<_, (String,)>("SELECT name FROM projects WHERE id=?")
            .bind(pid)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?
            .map(|(n,)| n)
    } else {
        None
    };

    let rows: Vec<ArchiveMsgRow> = sqlx::query_as(
        "SELECT m.from_agent, m.content_json, m.created_at, m.excluded_from_context,
                a.name AS agent_name, a.name_en AS agent_en, a.color AS agent_color
         FROM messages m
         LEFT JOIN agents a ON a.id = m.from_agent
         WHERE m.conversation_id = ?
         ORDER BY m.created_at ASC, m.id ASC",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Err("会议室没有可归档的内容".to_string());
    }

    let mut messages: Vec<ArchivedMessage> = Vec::with_capacity(rows.len());
    let mut search_text = String::new();
    for r in &rows {
        let is_me = r.from_agent.is_none();
        let is_innate = r.from_agent.as_deref() == Some(INNATE_SENDER);
        let author = if is_me {
            "我".to_string()
        } else if is_innate {
            "Innate".to_string()
        } else {
            r.agent_name.clone().unwrap_or_else(|| "Agent".to_string())
        };
        search_text.push_str(&extract_plain_text(&r.content_json));
        search_text.push('\n');
        messages.push(ArchivedMessage {
            from_agent: r.from_agent.clone(),
            author,
            author_en: r.agent_en.clone().unwrap_or_default(),
            author_color: r.agent_color.clone().unwrap_or_default(),
            is_me,
            is_innate,
            content_json: r.content_json.clone(),
            created_at: r.created_at.clone(),
            excluded_from_context: r.excluded_from_context,
        });
    }

    let payload_json = serde_json::to_string(&messages).map_err(|e| e.to_string())?;
    let archive_id = Uuid::new_v4().to_string();
    let message_count = messages.len() as i64;

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO conversation_archives
         (id, conversation_id, conv_type, title, project_id, project_name,
          message_count, payload_json, search_text, archived_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))",
    )
    .bind(&archive_id)
    .bind(&conversation_id)
    .bind(&conv_type)
    .bind(&title)
    .bind(&project_id)
    .bind(&project_name)
    .bind(message_count)
    .bind(&payload_json)
    .bind(&search_text)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    // 复用清空逻辑：删除消息/已读/附件/任务记录（房间本身保留）。
    let attachment_paths =
        crate::commands::conversations::purge_conversation_messages(&mut tx, &conversation_id)
            .await?;

    tx.commit().await.map_err(|e| e.to_string())?;
    crate::commands::conversations::remove_attachment_files(attachment_paths).await;

    let archived_at: (String,) =
        sqlx::query_as("SELECT archived_at FROM conversation_archives WHERE id=?")
            .bind(&archive_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    Ok(ConversationArchiveSummary {
        id: archive_id,
        conversation_id,
        conv_type,
        title,
        project_id,
        project_name,
        message_count,
        archived_at: archived_at.0,
    })
}

/// 归档列表（按归档时间倒序），不含完整消息快照。
#[tauri::command]
pub async fn list_conversation_archives(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationArchiveSummary>, String> {
    sqlx::query_as::<_, ConversationArchiveSummary>(
        "SELECT id, conversation_id, conv_type, title, project_id, project_name,
                message_count, archived_at
         FROM conversation_archives
         ORDER BY archived_at DESC, created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// 归档详情（含完整只读消息快照，供回顾渲染）。
#[tauri::command]
pub async fn get_conversation_archive(
    archive_id: String,
    state: State<'_, AppState>,
) -> Result<ConversationArchiveDetail, String> {
    sqlx::query_as::<_, ConversationArchiveDetail>(
        "SELECT id, conversation_id, conv_type, title, project_id, project_name,
                message_count, payload_json, archived_at
         FROM conversation_archives WHERE id=?",
    )
    .bind(&archive_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("archive {} not found", archive_id))
}

/// 对归档消息正文做全文检索，返回摘要 + 命中次数 + 片段（按命中数倒序）。
#[tauri::command]
pub async fn search_conversation_archives(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<ArchiveSearchHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(ConversationArchiveSummary, String)> = sqlx::query_as::<
        _,
        (String, String, String, String, Option<String>, Option<String>, i64, String, String),
    >(
        "SELECT id, conversation_id, conv_type, title, project_id, project_name,
                message_count, archived_at, search_text
         FROM conversation_archives
         WHERE lower(search_text) LIKE '%' || lower(?) || '%'
            OR lower(title) LIKE '%' || lower(?) || '%'
         ORDER BY archived_at DESC",
    )
    .bind(q)
    .bind(q)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|(id, conversation_id, conv_type, title, project_id, project_name, message_count, archived_at, search_text)| {
        (
            ConversationArchiveSummary {
                id,
                conversation_id,
                conv_type,
                title,
                project_id,
                project_name,
                message_count,
                archived_at,
            },
            search_text,
        )
    })
    .collect();

    let mut hits: Vec<ArchiveSearchHit> = rows
        .into_iter()
        .map(|(summary, search_text)| {
            let (match_count, snippet) = count_and_snippet(&search_text, q);
            ArchiveSearchHit {
                summary,
                match_count,
                snippet,
            }
        })
        .collect();
    hits.sort_by(|a, b| b.match_count.cmp(&a.match_count));
    Ok(hits)
}

/// 删除一条归档记录（归档内容只读，但允许整条移除以管理存储）。
#[tauri::command]
pub async fn delete_conversation_archive(
    archive_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM conversation_archives WHERE id=?")
        .bind(&archive_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
