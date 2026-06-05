mod agents;
mod commands;
mod core;
mod db;
mod intake;
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
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&data_dir).expect("create data dir");
            let db_path = data_dir.join("autoforge.db").to_string_lossy().to_string();
            let worktrees = data_dir.join("worktrees").to_string_lossy().to_string();
            let attachments = data_dir.join("attachments").to_string_lossy().to_string();
            let materials = data_dir.join("materials").to_string_lossy().to_string();
            state::init_worktrees_base(worktrees);
            state::init_attachments_base(attachments);
            state::init_materials_base(materials);

            let db = tauri::async_runtime::block_on(async {
                db::init(&db_path).await.expect("db init failed")
            });
            let (max_slots, pause_threshold, queue_strategy) =
                tauri::async_runtime::block_on(commands::system::load_concurrency_settings(&db))
                    .expect("load concurrency settings failed");

            let concurrency =
                core::concurrency::ConcurrencyManager::new(max_slots, pause_threshold);
            concurrency.update_config(None, None, Some(queue_strategy));
            let job_tx = tasks::runner::start(db.clone(), app_handle.clone(), concurrency.clone());

            let webhook_handle = std::sync::Arc::new(tokio::sync::Mutex::new(None));

            app.manage(AppState {
                db: db.clone(),
                job_tx: job_tx.clone(),
                concurrency,
                dev_servers: std::sync::Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                webhook_handle: webhook_handle.clone(),
            });

            // 启动 webhook server（若配置中已启用）
            let db_for_wh = db.clone();
            let app_for_wh = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(cfg) = sqlx::query_as::<_, models::intake::IntakeConfig>(
                    "SELECT * FROM intake_configs WHERE id='singleton'",
                )
                .fetch_one(&db_for_wh)
                .await
                {
                    if cfg.webhook_enabled && !cfg.webhook_token.is_empty() {
                        let port = cfg.webhook_port as u16;
                        let token = cfg.webhook_token.clone();
                        let db_clone = db_for_wh.clone();
                        let app_clone = app_for_wh.clone();
                        let handle = tokio::spawn(async move {
                            if let Err(e) = intake::webhook::start(
                                port,
                                token,
                                db_clone,
                                job_tx.clone(),
                                app_clone,
                            )
                            .await
                            {
                                tracing::error!("[webhook] server error: {}", e);
                            }
                        });
                        *webhook_handle.lock().await = Some(handle);
                    }
                }
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
            commands::projects::list_active_projects,
            commands::projects::get_project,
            commands::projects::create_project,
            commands::projects::update_project,
            commands::projects::delete_project,
            commands::issues::list_issues,
            commands::issues::get_issue,
            commands::issues::get_issue_analysis,
            commands::issues::submit_issue,
            commands::intake::get_intake_config,
            commands::intake::update_intake_config,
            commands::intake::get_webhook_status,
            commands::intake::sync_github_issues,
            commands::intake::run_code_scan,
            commands::intake::bulk_import_issues,
            commands::intake::submit_from_artifact,
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
            commands::project_context::list_project_files,
            commands::project_context::read_project_file,
            commands::project_context::list_conversation_project_context,
            commands::project_context::add_conversation_project_context,
            commands::project_context::remove_conversation_project_context,
            commands::workspace::ensure_workspace_dirs,
            commands::workspace::list_workspace_files,
            commands::workspace::read_workspace_file,
            commands::workspace::write_workspace_file,
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
            commands::demo::open_url,
            commands::dev_server::get_dev_server_status,
            commands::dev_server::start_dev_server,
            commands::dev_server::stop_dev_server,
            commands::materials::list_material_folders,
            commands::materials::create_material_folder,
            commands::materials::rename_material_folder,
            commands::materials::delete_material_folder,
            commands::materials::list_material_files,
            commands::materials::import_material_file,
            commands::materials::move_material_file,
            commands::materials::update_material_file_meta,
            commands::materials::delete_material_file,
            commands::materials::open_material_file,
            commands::materials::material_file_data_url,
            commands::materials::ai_organize_materials,
            commands::materials::get_material_backup_config,
            commands::materials::update_material_backup_config,
            commands::materials::backup_material_files,
            commands::specs::list_project_specs,
            commands::specs::upsert_project_spec,
            commands::specs::delete_project_spec,
            commands::specs::ai_generate_specs,
        ])
        .run(tauri::generate_context!())
        .expect("error running AutoForge");
}
