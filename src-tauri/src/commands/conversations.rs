use crate::core::event;
use crate::models::conversation::{
    AttachmentUpload, Conversation, ConversationAttachment, ConversationDetail, Message,
    SendMessage,
};
use crate::state::AppState;
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, State};
use tracing::{debug, info, warn};
use uuid::Uuid;

const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;

#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationDetail>, String> {
    let t0 = std::time::Instant::now();
    debug!("[cmd] list_conversations start");
    ensure_direct_conversations(&state.db).await?;

    let convs =
        sqlx::query_as::<_, Conversation>("SELECT * FROM conversations ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let member_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT conversation_id, agent_id
         FROM conversation_members
         ORDER BY conversation_id, agent_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let mut members_by_conversation: HashMap<String, Vec<String>> = HashMap::new();
    for (conversation_id, agent_id) in member_rows {
        members_by_conversation
            .entry(conversation_id)
            .or_default()
            .push(agent_id);
    }

    // Last message per conversation. The previous version used a correlated
    // subquery (`WHERE m.id = (SELECT id FROM messages WHERE conversation_id = m.conversation_id ...)`)
    // which is O(N×M) — for every message row SQLite re-ran the inner query.
    // Window function version is O(N), leveraging ix_messages_conv(conversation_id, created_at).
    let last_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT conversation_id, content_json, created_at
         FROM (
             SELECT conversation_id, content_json, created_at,
                    ROW_NUMBER() OVER (PARTITION BY conversation_id ORDER BY created_at DESC) AS rn
             FROM messages
         )
         WHERE rn = 1",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let last_by_conversation: HashMap<String, (String, String)> = last_rows
        .into_iter()
        .map(|(conversation_id, content_json, created_at)| {
            (conversation_id, (content_json, created_at))
        })
        .collect();

    let unread_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT m.conversation_id, COUNT(*)
         FROM messages m
         LEFT JOIN conversation_reads r ON r.conversation_id = m.conversation_id
         WHERE m.from_agent IS NOT NULL
           AND m.created_at > COALESCE(r.read_at, '1970-01-01')
         GROUP BY m.conversation_id",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let unread_by_conversation: HashMap<String, i64> = unread_rows.into_iter().collect();

    let mut details = Vec::new();
    for conv in convs {
        let member_ids = members_by_conversation
            .get(&conv.id)
            .cloned()
            .unwrap_or_default();

        if conv.conv_type == "direct"
            && member_ids.is_empty()
            && !last_by_conversation.contains_key(&conv.id)
        {
            continue;
        }

        let (last_message, last_time) = match last_by_conversation.get(&conv.id).cloned() {
            Some((msg, time)) => (Some(msg), Some(time)),
            None => (None, None),
        };

        let unread = *unread_by_conversation.get(&conv.id).unwrap_or(&0);

        details.push(ConversationDetail {
            id: conv.id,
            conv_type: conv.conv_type,
            name: conv.name,
            color: conv.color,
            initial: conv.initial,
            created_at: conv.created_at,
            members: member_ids,
            unread,
            last_message,
            last_time,
            project_id: conv.project_id,
        });
    }

    info!(
        "[cmd] list_conversations done in {:?} ({} convs)",
        t0.elapsed(),
        details.len()
    );
    Ok(details)
}

async fn ensure_direct_conversations(db: &crate::db::Db) -> Result<(), String> {
    let missing_agents: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT a.id, a.color, a.initial
         FROM agents a
         WHERE a.visible_in_chat = 1
           AND a.enabled = 1
           AND NOT EXISTS (
             SELECT 1
             FROM conversation_members cm
             JOIN conversations c ON c.id=cm.conversation_id
             WHERE cm.agent_id=a.id AND c.type='direct'
         )",
    )
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;

    if !missing_agents.is_empty() {
        info!(
            "[cmd] ensure_direct_conversations: creating {} missing direct convs",
            missing_agents.len()
        );
    }
    for (agent_id, color, initial) in missing_agents {
        let conversation_id = format!("conv-direct-{}", agent_id);
        sqlx::query(
            "INSERT OR IGNORE INTO conversations (id, type, name, color, initial) VALUES (?, 'direct', NULL, ?, ?)",
        )
        .bind(&conversation_id)
        .bind(&color)
        .bind(&initial)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT OR IGNORE INTO conversation_members (conversation_id, agent_id) VALUES (?, ?)",
        )
        .bind(&conversation_id)
        .bind(&agent_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn list_messages(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Message>, String> {
    debug!("[cmd] list_messages conv={}", conversation_id);
    sqlx::query_as::<_, Message>(
        "SELECT *
         FROM (
             SELECT *
             FROM messages
             WHERE conversation_id=?
             ORDER BY created_at DESC
             LIMIT 300
         )
         ORDER BY created_at ASC",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_message(
    payload: SendMessage,
    state: State<'_, AppState>,
) -> Result<Message, String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json) VALUES (?, ?, NULL, ?)"
    )
    .bind(&id)
    .bind(&payload.conversation_id)
    .bind(&payload.content_json)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug)]
struct AttachmentPolicy {
    ext: &'static str,
    mime: &'static str,
    kind: &'static str,
}

#[tauri::command]
pub async fn import_attachment(
    payload: AttachmentUpload,
    state: State<'_, AppState>,
) -> Result<ConversationAttachment, String> {
    let conversation_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM conversations WHERE id=?")
            .bind(&payload.conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    if conversation_exists.is_none() {
        return Err(format!(
            "conversation {} not found",
            payload.conversation_id
        ));
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
    let rel_path = format!("{}/{}", payload.conversation_id, stored_name);
    let base = PathBuf::from(crate::state::attachments_base());
    let conversation_dir = base.join(&payload.conversation_id);
    let file_path = conversation_dir.join(&stored_name);

    tokio::fs::create_dir_all(&conversation_dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| e.to_string())?;

    let sha256 = hex::encode(Sha256::digest(&bytes));
    let insert_result = sqlx::query(
        "INSERT INTO conversation_attachments
         (id, conversation_id, original_name, stored_name, rel_path, mime, kind, size_bytes, sha256)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&attachment_id)
    .bind(&payload.conversation_id)
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

    sqlx::query_as::<_, ConversationAttachment>("SELECT * FROM conversation_attachments WHERE id=?")
        .bind(&attachment_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_conversation_attachments(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ConversationAttachment>, String> {
    let conversation_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM conversations WHERE id=?")
            .bind(&conversation_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    if conversation_exists.is_none() {
        return Err(format!("conversation {} not found", conversation_id));
    }

    sqlx::query_as::<_, ConversationAttachment>(
        "SELECT *
         FROM conversation_attachments
         WHERE conversation_id=?
         ORDER BY created_at DESC",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn attachment_data_url(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let attachment = load_attachment(&state.db, &attachment_id).await?;
    if attachment.kind != "image" {
        return Err("只有图片附件支持内联预览".to_string());
    }
    if attachment.size_bytes as usize > MAX_ATTACHMENT_BYTES {
        return Err("图片超过预览大小上限".to_string());
    }
    let path = attachment_path(&attachment)?;
    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{};base64,{}",
        attachment.mime,
        general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub async fn open_attachment(
    attachment_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let attachment = load_attachment(&state.db, &attachment_id).await?;
    let path = attachment_path(&attachment)?;
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

async fn load_attachment(
    db: &crate::db::Db,
    attachment_id: &str,
) -> Result<ConversationAttachment, String> {
    sqlx::query_as::<_, ConversationAttachment>("SELECT * FROM conversation_attachments WHERE id=?")
        .bind(attachment_id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "附件不存在".to_string())
}

fn attachment_path(attachment: &ConversationAttachment) -> Result<PathBuf, String> {
    let rel = Path::new(&attachment.rel_path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("附件路径无效".to_string());
    }
    Ok(PathBuf::from(crate::state::attachments_base()).join(rel))
}

fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment");
    let mut cleaned = String::with_capacity(base.len());
    for ch in base.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let cleaned = cleaned
        .trim_matches([' ', '.'])
        .chars()
        .take(120)
        .collect::<String>();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

fn validate_attachment(
    name: &str,
    mime_hint: &str,
    bytes: &[u8],
) -> Result<AttachmentPolicy, String> {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let policy = match ext.as_str() {
        "png" if is_png(bytes) => AttachmentPolicy {
            ext: "png",
            mime: "image/png",
            kind: "image",
        },
        "jpg" | "jpeg" if is_jpeg(bytes) => AttachmentPolicy {
            ext: "jpg",
            mime: "image/jpeg",
            kind: "image",
        },
        "webp" if is_webp(bytes) => AttachmentPolicy {
            ext: "webp",
            mime: "image/webp",
            kind: "image",
        },
        "gif" if is_gif(bytes) => AttachmentPolicy {
            ext: "gif",
            mime: "image/gif",
            kind: "image",
        },
        "pdf" if bytes.starts_with(b"%PDF-") => AttachmentPolicy {
            ext: "pdf",
            mime: "application/pdf",
            kind: "file",
        },
        "txt" | "log" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "txt",
            mime: "text/plain",
            kind: "file",
        },
        "md" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "md",
            mime: "text/markdown",
            kind: "file",
        },
        "csv" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "csv",
            mime: "text/csv",
            kind: "file",
        },
        "json"
            if is_safe_text(bytes)
                && serde_json::from_slice::<serde_json::Value>(bytes).is_ok() =>
        {
            AttachmentPolicy {
                ext: "json",
                mime: "application/json",
                kind: "file",
            }
        }
        "yaml" | "yml" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "yaml",
            mime: "application/x-yaml",
            kind: "file",
        },
        "toml" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "toml",
            mime: "application/toml",
            kind: "file",
        },
        _ => {
            return Err(
                "不支持的附件类型。允许：PNG/JPG/WebP/GIF/PDF/TXT/MD/JSON/CSV/YAML/TOML，禁止脚本、HTML、SVG、压缩包和可执行文件。"
                    .to_string(),
            );
        }
    };

    if !mime_hint.is_empty() && !mime_hint_matches(mime_hint, policy.mime) {
        return Err("文件扩展名、浏览器 MIME 和内容特征不一致".to_string());
    }

    Ok(policy)
}

fn mime_hint_matches(hint: &str, canonical: &str) -> bool {
    let hint = hint
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if hint.is_empty() || hint == "application/octet-stream" {
        return true;
    }
    matches!(
        (hint.as_str(), canonical),
        ("image/jpeg", "image/jpeg")
            | ("image/png", "image/png")
            | ("image/webp", "image/webp")
            | ("image/gif", "image/gif")
            | ("application/pdf", "application/pdf")
            | ("text/plain", "text/plain")
            | ("text/plain", "text/markdown")
            | ("text/markdown", "text/markdown")
            | ("text/csv", "text/csv")
            | ("application/json", "application/json")
            | ("text/yaml", "application/x-yaml")
            | ("application/x-yaml", "application/x-yaml")
            | ("application/toml", "application/toml")
    )
}

fn is_safe_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0..3] == [0xff, 0xd8, 0xff]
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

#[tauri::command]
pub async fn create_group_conversation(
    name: String,
    member_ids: Vec<String>,
    color: Option<String>,
    initial: Option<String>,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let id = Uuid::new_v4().to_string();
    let color = color.unwrap_or_else(|| "#e8772e".to_string());

    // Validate project if provided
    if let Some(ref pid) = project_id {
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
            .bind(pid)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("project {} not found", pid));
        }
    }

    sqlx::query(
        "INSERT INTO conversations (id, type, name, color, initial, project_id) VALUES (?, 'group', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&color)
    .bind(&initial)
    .bind(&project_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for agent_id in &member_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO conversation_members (conversation_id, agent_id) VALUES (?, ?)",
        )
        .bind(&id)
        .bind(agent_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(ConversationDetail {
        id: id.clone(),
        conv_type: "group".to_string(),
        name: Some(name),
        color,
        initial,
        created_at: chrono::Utc::now().to_rfc3339(),
        members: member_ids,
        unread: 0,
        last_message: None,
        last_time: None,
        project_id,
    })
}

#[tauri::command]
pub async fn update_group_conversation(
    conversation_id: String,
    name: String,
    project_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;
    if conv.conv_type != "group" {
        return Err("only group conversations can be edited".into());
    }

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("group name cannot be empty".into());
    }

    if let Some(ref pid) = project_id {
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id=?")
            .bind(pid)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("project {} not found", pid));
        }
    }

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE conversations SET name=?, initial=?, project_id=? WHERE id=?")
        .bind(&name)
        .bind(name.chars().next().map(|c| c.to_string()))
        .bind(&project_id)
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    if conv.project_id != project_id {
        sqlx::query("DELETE FROM conversation_project_context WHERE conversation_id=?")
            .bind(&conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    let updated = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    conversation_detail(&state.db, updated).await
}

#[tauri::command]
pub async fn add_conversation_member(
    conversation_id: String,
    agent_id: String,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;

    let agent_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM agents WHERE id=?")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if agent_exists.is_none() {
        return Err(format!("agent {} not found", agent_id));
    }

    sqlx::query(
        "INSERT OR IGNORE INTO conversation_members (conversation_id, agent_id) VALUES (?, ?)",
    )
    .bind(&conversation_id)
    .bind(&agent_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    conversation_detail(&state.db, conv).await
}

#[tauri::command]
pub async fn remove_conversation_member(
    conversation_id: String,
    agent_id: String,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;

    let (member_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM conversation_members WHERE conversation_id=?")
            .bind(&conversation_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    if conv.conv_type == "group" && member_count <= 2 {
        return Err("群聊至少保留 2 个成员".to_string());
    }
    if conv.conv_type == "direct" {
        return Err("单聊成员不可删除".to_string());
    }

    sqlx::query("DELETE FROM conversation_members WHERE conversation_id=? AND agent_id=?")
        .bind(&conversation_id)
        .bind(&agent_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    conversation_detail(&state.db, conv).await
}

#[tauri::command]
pub async fn delete_group_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("conversation {} not found", conversation_id))?;

    if conv.conv_type != "group" {
        return Err("只能解散群聊".to_string());
    }

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

    let attachment_paths: Vec<(String,)> =
        sqlx::query_as("SELECT rel_path FROM conversation_attachments WHERE conversation_id=?")
            .bind(&conversation_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

    delete_conversation_task_records(&mut tx, &conversation_id).await?;

    sqlx::query("DELETE FROM messages WHERE conversation_id=?")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_reads WHERE conversation_id=?")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_members WHERE conversation_id=?")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_attachments WHERE conversation_id=?")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    let base = PathBuf::from(crate::state::attachments_base());
    for (rel_path,) in attachment_paths {
        let rel = Path::new(&rel_path);
        if rel.components().all(|c| matches!(c, Component::Normal(_))) {
            let _ = tokio::fs::remove_file(base.join(rel)).await;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn clear_conversation_messages(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("conversation {} not found", conversation_id));
    }

    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
    let attachment_paths = purge_conversation_messages(&mut tx, &conversation_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    remove_attachment_files(attachment_paths).await;
    Ok(())
}

/// 清空一个会议室的全部消息/已读/附件记录及关联任务记录（在调用方的事务内执行）。
/// 返回被删除附件的相对路径，供提交后异步清理磁盘文件。归档与清空共用此逻辑。
pub(crate) async fn purge_conversation_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation_id: &str,
) -> Result<Vec<(String,)>, String> {
    let attachment_paths: Vec<(String,)> =
        sqlx::query_as("SELECT rel_path FROM conversation_attachments WHERE conversation_id=?")
            .bind(conversation_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;

    delete_conversation_task_records(tx, conversation_id).await?;

    sqlx::query("DELETE FROM messages WHERE conversation_id=?")
        .bind(conversation_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_reads WHERE conversation_id=?")
        .bind(conversation_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_attachments WHERE conversation_id=?")
        .bind(conversation_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

    Ok(attachment_paths)
}

/// 提交事务后，按相对路径删除磁盘上的附件文件（路径越界则跳过）。
pub(crate) async fn remove_attachment_files(attachment_paths: Vec<(String,)>) {
    let base = PathBuf::from(crate::state::attachments_base());
    for (rel_path,) in attachment_paths {
        let rel = Path::new(&rel_path);
        if rel.components().all(|c| matches!(c, Component::Normal(_))) {
            let _ = tokio::fs::remove_file(base.join(rel)).await;
        }
    }
}

async fn delete_conversation_task_records(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation_id: &str,
) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM conversation_task_runs
         WHERE task_id IN (
             SELECT id FROM conversation_tasks WHERE conversation_id=?
         )
            OR message_id IN (
             SELECT id FROM messages WHERE conversation_id=?
         )",
    )
    .bind(conversation_id)
    .bind(conversation_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM conversation_task_steps
         WHERE task_id IN (
             SELECT id FROM conversation_tasks WHERE conversation_id=?
         )",
    )
    .bind(conversation_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversation_tasks WHERE conversation_id=?")
        .bind(conversation_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn mark_conversation_read(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    debug!("[cmd] mark_conversation_read conv={}", conversation_id);
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM conversations WHERE id=?")
        .bind(&conversation_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("conversation {} not found", conversation_id));
    }

    sqlx::query(
        "INSERT INTO conversation_reads (conversation_id, read_at)
         VALUES (?, datetime('now'))
         ON CONFLICT(conversation_id) DO UPDATE SET read_at=excluded.read_at",
    )
    .bind(&conversation_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

async fn conversation_detail(
    db: &crate::db::Db,
    conv: Conversation,
) -> Result<ConversationDetail, String> {
    let members: Vec<(String,)> =
        sqlx::query_as("SELECT agent_id FROM conversation_members WHERE conversation_id=?")
            .bind(&conv.id)
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
    let member_ids: Vec<String> = members.into_iter().map(|(id,)| id).collect();

    let last: Option<(String, String)> = sqlx::query_as(
        "SELECT content_json, created_at FROM messages WHERE conversation_id=? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&conv.id)
    .fetch_optional(db)
    .await
    .map_err(|e| e.to_string())?;

    let (last_message, last_time) = match last {
        Some((msg, time)) => (Some(msg), Some(time)),
        None => (None, None),
    };

    let unread = unread_count(db, &conv.id).await?;

    Ok(ConversationDetail {
        id: conv.id,
        conv_type: conv.conv_type,
        name: conv.name,
        color: conv.color,
        initial: conv.initial,
        created_at: conv.created_at,
        members: member_ids,
        unread,
        last_message,
        last_time,
        project_id: conv.project_id,
    })
}

/// Read a text-based attachment from disk and return its content.
/// Returns `Ok(None)` for PDF/binary mimes that cannot be inlined as text.
async fn load_text_attachment_content(
    attachment_id: &str,
    db: &crate::db::Db,
) -> Result<Option<String>, String> {
    let attachment = load_attachment(db, attachment_id).await?;
    const TEXT_MIMES: &[&str] = &[
        "text/plain",
        "text/markdown",
        "text/csv",
        "application/json",
        "application/x-yaml",
        "application/toml",
    ];
    if !TEXT_MIMES.contains(&attachment.mime.as_str()) {
        return Ok(None);
    }
    let path = attachment_path(&attachment)?;
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "文件不是有效的 UTF-8 文本".to_string())?;
    // Truncate huge files so we don't blow up the context window.
    const MAX_CHARS: usize = 50_000;
    if text.chars().count() > MAX_CHARS {
        let truncated: String = text.chars().take(MAX_CHARS).collect();
        Ok(Some(format!(
            "{}\n\n[… 内容已截断，原始大小 {} 字节]",
            truncated,
            bytes.len()
        )))
    } else {
        Ok(Some(text.to_string()))
    }
}

/// Return the filesystem path for an image attachment, verifying it exists.
async fn get_attachment_file_path(
    attachment_id: &str,
    db: &crate::db::Db,
) -> Result<PathBuf, String> {
    let attachment = load_attachment(db, attachment_id).await?;
    let path = attachment_path(&attachment)?;
    if !path.exists() {
        return Err("图片文件不存在".to_string());
    }
    Ok(path)
}

async fn unread_count(db: &crate::db::Db, conversation_id: &str) -> Result<i64, String> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM messages
         WHERE conversation_id=?
           AND from_agent IS NOT NULL
           AND created_at > COALESCE(
             (SELECT read_at FROM conversation_reads WHERE conversation_id=?),
             '1970-01-01'
           )",
    )
    .bind(conversation_id)
    .bind(conversation_id)
    .fetch_one(db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(count)
}

#[tauri::command]
pub async fn toggle_message_context(
    message_id: String,
    state: State<'_, AppState>,
) -> Result<Message, String> {
    sqlx::query("UPDATE messages SET excluded_from_context = NOT excluded_from_context WHERE id=?")
        .bind(&message_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id=?")
        .bind(&message_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn agent_reply(
    conversation_id: String,
    agent_id: String,
    window_size: Option<i64>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Message, String> {
    let t0 = std::time::Instant::now();
    let limit = window_size.unwrap_or(20).clamp(5, 100);
    info!(
        "[cmd] agent_reply conv={} agent={} window={}",
        conversation_id, agent_id, limit
    );

    let agent = sqlx::query_as::<_, crate::models::agent::Agent>("SELECT * FROM agents WHERE id=?")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("agent {} not found", agent_id))?;

    // Fetch the window, then exclude flagged messages (keep in DESC order for the limit,
    // then reverse to chronological for the prompt).
    let candidates = sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(&conversation_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let messages: Vec<Message> = candidates
        .into_iter()
        .filter(|m| !m.excluded_from_context)
        .collect();

    // Build prompt from message blocks, extracting file content and collecting image paths.
    let mut prompt_parts: Vec<String> = Vec::new();
    let mut image_paths: Vec<PathBuf> = Vec::new();

    for msg in messages.iter().rev() {
        let sender = msg.from_agent.as_deref().unwrap_or("User");
        let blocks: Vec<serde_json::Value> = serde_json::from_str(&msg.content_json)
            .unwrap_or_else(|_| vec![serde_json::json!({"t": "md", "md": msg.content_json})]);

        let mut parts: Vec<String> = Vec::new();
        for block in &blocks {
            match block.get("t").and_then(|t| t.as_str()) {
                Some("md") => {
                    if let Some(md) = block.get("md").and_then(|v| v.as_str()) {
                        parts.push(md.to_string());
                    }
                }
                Some("file") => {
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("文件");
                    if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                        match load_text_attachment_content(id, &state.db).await {
                            Ok(Some(content)) => {
                                parts.push(format!("[文件内容 — {}]\n```\n{}\n```", name, content));
                            }
                            Ok(None) => {
                                parts
                                    .push(format!("[附件: {} (PDF 或二进制，无法提取文本)]", name));
                            }
                            Err(e) => {
                                parts.push(format!("[附件: {} (读取失败: {})]", name, e));
                            }
                        }
                    } else {
                        parts.push(format!("[附件: {}]", name));
                    }
                }
                Some("image") => {
                    let label = block
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("图片");
                    if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                        match get_attachment_file_path(id, &state.db).await {
                            Ok(path) => {
                                parts.push(format!("[图片: {}]", label));
                                image_paths.push(path);
                            }
                            Err(_) => {
                                parts.push(format!("[图片: {} (文件不可用)]", label));
                            }
                        }
                    } else {
                        parts.push(format!("[图片: {}]", label));
                    }
                }
                Some("quote_ref") => {
                    let author = block.get("author").and_then(|v| v.as_str()).unwrap_or("?");
                    let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    parts.push(format!("[引用 {}]: {}", author, text));
                }
                Some("code") => {
                    let lang = block.get("lang").and_then(|v| v.as_str()).unwrap_or("");
                    let code = block.get("code").and_then(|v| v.as_str()).unwrap_or("");
                    parts.push(format!("```{}\n{}\n```", lang, code));
                }
                Some("artifact") => {
                    let title = block
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("报告");
                    let body = block.get("body").and_then(|v| v.as_str()).unwrap_or("");
                    parts.push(format!("[分析报告 — {}]\n{}", title, body));
                }
                _ => {}
            }
        }

        if !parts.is_empty() {
            prompt_parts.push(format!("[{}]: {}", sender, parts.join("\n")));
        }
    }

    let context = prompt_parts.join("\n\n");
    let prompt = format!(
        "以下是对话历史：\n{}\n\n请以 {} 的身份回复最后一条消息。",
        context, agent.name
    );

    let system_prompt = if agent.system_prompt.is_empty() {
        None
    } else {
        Some(agent.system_prompt.as_str())
    };

    info!(
        "[cmd] agent_reply: calling claude CLI with {} image(s) (elapsed {:?})",
        image_paths.len(),
        t0.elapsed()
    );
    // 构建该 Agent 的工具集：内置工具(capabilities 白名单) + 勾选了它的 MCP server 工具。
    let registry = crate::agents::tools::build_registry_for_agent(&state.db, &agent).await;
    let reply_text = crate::agents::llm::run_agent_text_with_tools(
        &state.db,
        &agent,
        &prompt,
        system_prompt,
        &image_paths,
        &registry,
    )
    .await
    .unwrap_or_else(|e| {
        warn!("[cmd] agent_reply: agent LLM error: {}", e);
        format!("[系统错误: {}]", e)
    });
    info!(
        "[cmd] agent_reply: claude CLI returned in {:?}",
        t0.elapsed()
    );

    // Wrap reply in content_json format
    let content_json = serde_json::json!([{"t": "md", "md": reply_text}]).to_string();

    // Persist reply
    let msg_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, from_agent, content_json) VALUES (?, ?, ?, ?)",
    )
    .bind(&msg_id)
    .bind(&conversation_id)
    .bind(&agent_id)
    .bind(&content_json)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Emit event
    event::emit(
        &app,
        event::AppEvent::MessageReceived {
            conversation_id: conversation_id.clone(),
            message_id: msg_id.clone(),
        },
    );

    let result = sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id=?")
        .bind(&msg_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string());
    info!("[cmd] agent_reply done in {:?}", t0.elapsed());
    result
}
