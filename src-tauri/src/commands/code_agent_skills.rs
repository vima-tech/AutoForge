//! 编码 Agent 技能（Skill）的 Tauri 命令（薄包装：取 state → 调 sqlx → 返回）。
//! 注入逻辑在 `agents::code_agent::{load_code_agent_skills, skill_inject}`；这里只做 CRUD。
//! 技能不含密钥，无需 secrets 加密。`project_id` 为 NULL 表示全局适用。
use crate::models::code_agent_skill::{CodeAgentSkillRow, UpsertCodeAgentSkill};
use crate::state::AppState;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_code_agent_skills(
    state: State<'_, AppState>,
) -> Result<Vec<CodeAgentSkillRow>, String> {
    sqlx::query_as::<_, CodeAgentSkillRow>(
        "SELECT * FROM code_agent_skills ORDER BY created_at ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upsert_code_agent_skill(
    payload: UpsertCodeAgentSkill,
    state: State<'_, AppState>,
) -> Result<CodeAgentSkillRow, String> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err("技能名不能为空".into());
    }
    let project_id = payload.project_id.as_deref().filter(|s| !s.is_empty());

    let id = match payload.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            sqlx::query(
                "UPDATE code_agent_skills
                 SET name=?, description=?, body=?, project_id=?, enabled=?, updated_at=datetime('now')
                 WHERE id=?",
            )
            .bind(name)
            .bind(&payload.description)
            .bind(&payload.body)
            .bind(project_id)
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
                "INSERT INTO code_agent_skills (id, name, description, body, project_id, enabled)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(name)
            .bind(&payload.description)
            .bind(&payload.body)
            .bind(project_id)
            .bind(payload.enabled)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            id
        }
    };

    sqlx::query_as::<_, CodeAgentSkillRow>("SELECT * FROM code_agent_skills WHERE id=?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_code_agent_skill(id: String, state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM code_agent_skills WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
