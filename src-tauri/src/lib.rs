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
            let attachments = data_dir.join("attachments").to_string_lossy().to_string();
            state::init_worktrees_base(worktrees);
            state::init_attachments_base(attachments);

            let db = tauri::async_runtime::block_on(async {
                db::init(&db_path).await.expect("db init failed")
            });
            let (max_slots, pause_threshold, queue_strategy) =
                tauri::async_runtime::block_on(commands::system::load_concurrency_settings(&db))
                    .expect("load concurrency settings failed");

            let concurrency =
                core::concurrency::ConcurrencyManager::new(max_slots, pause_threshold);
            concurrency.update_config(None, None, Some(queue_strategy));
            let job_tx = tasks::runner::start(db.clone(), app_handle, concurrency.clone());

            app.manage(AppState {
                db,
                job_tx,
                concurrency,
                dev_servers: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            });

            // 显式设置窗口图标，确保 Linux 任务栏在开发模式下也能显示正确图标
            if let Some(win) = app.get_webview_window("main") {
                let icon_bytes = include_bytes!("../icons/icon.png");
                if let Ok(icon) = tauri::image::Image::from_bytes(icon_bytes) {
                    let _ = win.set_icon(icon);
                }
            }

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
            commands::conversations::import_attachment,
            commands::conversations::list_conversation_attachments,
            commands::conversations::open_attachment,
            commands::conversations::attachment_data_url,
            commands::conversations::create_group_conversation,
            commands::conversations::add_conversation_member,
            commands::conversations::remove_conversation_member,
            commands::conversations::delete_group_conversation,
            commands::conversations::clear_conversation_messages,
            commands::conversations::mark_conversation_read,
            commands::conversations::agent_reply,
            commands::conversations::toggle_message_context,
            commands::orchestration::start_conversation_task,
            commands::orchestration::list_conversation_tasks,
            commands::settings::list_llm_configs,
            commands::settings::create_llm_config,
            commands::settings::update_llm_config,
            commands::settings::delete_llm_config,
            commands::settings::test_llm_connection,
            commands::settings::list_agents,
            commands::settings::create_agent,
            commands::settings::update_agent,
            commands::settings::delete_agent,
            commands::settings::set_agent_forge_role,
            commands::system::system_health,
            commands::system::check_claude_auth,
            commands::system::pipeline_stats,
            commands::system::get_badge_counts,
            commands::system::read_spec,
            commands::system::write_spec,
            commands::system::update_concurrency_config,
            commands::system::get_concurrency_config,
            commands::system::list_preview_environments,
            commands::system::list_test_sessions,
            commands::system::list_scan_findings,
            commands::system::list_admin_decisions,
            commands::demo::seed_demo_data,
            commands::demo::open_url,
            commands::dev_server::get_dev_server_status,
            commands::dev_server::start_dev_server,
            commands::dev_server::stop_dev_server,
        ])
        .run(tauri::generate_context!())
        .expect("error running AutoForge");
}
