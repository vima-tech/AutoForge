use crate::core::event;
use crate::models::conversation::{Conversation, ConversationDetail, Message, SendMessage};
use crate::state::AppState;
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationDetail>, String> {
    let convs =
        sqlx::query_as::<_, Conversation>("SELECT * FROM conversations ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    let mut details = Vec::new();
    for conv in convs {
        // Get members
        let members: Vec<(String,)> =
            sqlx::query_as("SELECT agent_id FROM conversation_members WHERE conversation_id=?")
                .bind(&conv.id)
                .fetch_all(&state.db)
                .await
                .unwrap_or_default();

        let member_ids: Vec<String> = members.into_iter().map(|(id,)| id).collect();

        // Last message
        let last: Option<(String, String)> = sqlx::query_as(
            "SELECT content_json, created_at FROM messages WHERE conversation_id=? ORDER BY created_at DESC LIMIT 1"
        )
        .bind(&conv.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

        let (last_message, last_time) = match last {
            Some((msg, time)) => (Some(msg), Some(time)),
            None => (None, None),
        };

        details.push(ConversationDetail {
            id: conv.id,
            conv_type: conv.conv_type,
            name: conv.name,
            color: conv.color,
            initial: conv.initial,
            created_at: conv.created_at,
            members: member_ids,
            unread: 0,
            last_message,
            last_time,
        });
    }

    Ok(details)
}

#[tauri::command]
pub async fn list_messages(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Message>, String> {
    sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at ASC",
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

#[tauri::command]
pub async fn create_group_conversation(
    name: String,
    member_ids: Vec<String>,
    color: Option<String>,
    initial: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let id = Uuid::new_v4().to_string();
    let color = color.unwrap_or_else(|| "#e8772e".to_string());

    sqlx::query(
        "INSERT INTO conversations (id, type, name, color, initial) VALUES (?, 'group', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&color)
    .bind(&initial)
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
    })
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

    Ok(ConversationDetail {
        id: conv.id,
        conv_type: conv.conv_type,
        name: conv.name,
        color: conv.color,
        initial: conv.initial,
        created_at: conv.created_at,
        members: member_ids,
        unread: 0,
        last_message,
        last_time,
    })
}

#[tauri::command]
pub async fn agent_reply(
    conversation_id: String,
    agent_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Message, String> {
    // Load agent info
    let agent = sqlx::query_as::<_, crate::models::agent::Agent>("SELECT * FROM agents WHERE id=?")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("agent {} not found", agent_id))?;

    // Load last 10 messages as context
    let messages = sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE conversation_id=? ORDER BY created_at DESC LIMIT 10",
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Build prompt from messages (reversed to chronological order)
    let mut prompt_parts = Vec::new();
    for msg in messages.iter().rev() {
        let sender = msg.from_agent.as_deref().unwrap_or("User");
        prompt_parts.push(format!("[{}]: {}", sender, msg.content_json));
    }
    let context = prompt_parts.join("\n");
    let prompt = format!(
        "以下是对话历史：\n{}\n\n请以 {} 的身份回复最后一条消息。",
        context, agent.name
    );

    // Call claude
    let system_prompt = if agent.system_prompt.is_empty() {
        None
    } else {
        Some(agent.system_prompt.as_str())
    };

    let reply_text = crate::agents::local_claude::run_text(&prompt, system_prompt)
        .await
        .unwrap_or_else(|e| format!("[系统错误: {}]", e));

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

    sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id=?")
        .bind(&msg_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}
