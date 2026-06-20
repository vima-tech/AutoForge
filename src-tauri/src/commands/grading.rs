use crate::models::cr_grade::{AutoPassPolicy, CrGrade};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_cr_grade(
    cr_id: String,
    state: State<'_, AppState>,
) -> Result<Option<CrGrade>, String> {
    sqlx::query_as::<_, CrGrade>("SELECT * FROM cr_grades WHERE change_request_id=?")
        .bind(&cr_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_auto_pass_policy(
    state: State<'_, AppState>,
) -> Result<Vec<AutoPassPolicy>, String> {
    sqlx::query_as::<_, AutoPassPolicy>("SELECT * FROM auto_pass_policy ORDER BY change_class")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// §7 global kill-switch for gate downgrade (auto-pass). OFF by default.
#[tauri::command]
pub async fn get_auto_pass_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(crate::core::gate::auto_pass_enabled(&state.db).await)
}

#[tauri::command]
pub async fn set_auto_pass_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::core::gate::set_auto_pass_enabled(&state.db, enabled)
        .await
        .map_err(|e| e.to_string())
}

/// 自动 AI 解冲突开关（与门控降级并排放在 Settings 门控降级面板）。OFF by default.
#[tauri::command]
pub async fn get_auto_conflict_resolve_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(crate::core::gate::auto_conflict_resolve_enabled(&state.db).await)
}

#[tauri::command]
pub async fn set_auto_conflict_resolve_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::core::gate::set_auto_conflict_resolve_enabled(&state.db, enabled)
        .await
        .map_err(|e| e.to_string())
}
