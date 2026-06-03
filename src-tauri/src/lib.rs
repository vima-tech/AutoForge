mod agents;
mod commands;
mod core;
mod db;
mod models;
mod state;
mod tasks;

pub use state::AppState;

use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&data_dir).expect("create data dir");
            let db_path = data_dir.join("autoforge.db").to_string_lossy().to_string();
            let worktrees = data_dir.join("worktrees").to_string_lossy().to_string();
            state::init_worktrees_base(worktrees);

            let db = tauri::async_runtime::block_on(async {
                db::init(&db_path).await.expect("db init failed")
            });

            let concurrency = core::concurrency::ConcurrencyManager::new(5, 20);
            let job_tx = tasks::runner::start(db.clone(), app_handle, concurrency.clone());

            app.manage(AppState {
                db,
                job_tx,
                concurrency,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::get_project,
            commands::projects::create_project,
            commands::projects::update_project,
            commands::projects::delete_project,
            commands::issues::list_issues,
            commands::issues::get_issue,
            commands::issues::get_issue_analysis,
            commands::issues::submit_issue,
            commands::change_requests::list_change_requests,
            commands::change_requests::get_change_request,
            commands::change_requests::get_worktree_session,
            commands::change_requests::get_code_diff,
            commands::change_requests::review_1,
            commands::change_requests::review_2,
            commands::conversations::list_conversations,
            commands::conversations::list_messages,
            commands::conversations::send_message,
            commands::conversations::create_group_conversation,
            commands::conversations::add_conversation_member,
            commands::conversations::remove_conversation_member,
            commands::conversations::agent_reply,
            commands::settings::list_llm_configs,
            commands::settings::create_llm_config,
            commands::settings::update_llm_config,
            commands::settings::delete_llm_config,
            commands::settings::test_llm_connection,
            commands::settings::list_agents,
            commands::settings::create_agent,
            commands::settings::update_agent,
            commands::settings::delete_agent,
            commands::system::system_health,
            commands::system::pipeline_stats,
            commands::system::read_spec,
            commands::system::write_spec,
            commands::system::update_concurrency_config,
            commands::system::list_preview_environments,
            commands::system::list_test_sessions,
            commands::system::list_scan_findings,
            commands::system::list_admin_decisions,
        ])
        .run(tauri::generate_context!())
        .expect("error running autoforge");
}
