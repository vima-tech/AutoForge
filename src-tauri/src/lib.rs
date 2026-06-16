mod agents;
mod commands;
mod core;
mod db;
mod intake;
mod knowledge;
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
            let kb = data_dir.join("kb").to_string_lossy().to_string();
            state::init_worktrees_base(worktrees);
            state::init_attachments_base(attachments);
            state::init_materials_base(materials);
            state::init_kb_base(kb);

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

            // 主动巡检调度器（design §6.2 mode B）：每 24h 对活跃项目跑全量巡检
            let db_for_scan = db.clone();
            let tx_for_scan = job_tx.clone();
            let app_for_scan = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
                    if let Ok(projects) = sqlx::query_as::<_, (String,)>(
                        "SELECT id FROM projects WHERE status='active'",
                    )
                    .fetch_all(&db_for_scan)
                    .await
                    {
                        for (pid,) in projects {
                            let _ = tasks::scan::run_proactive(
                                &db_for_scan,
                                &tx_for_scan,
                                &app_for_scan,
                                &pid,
                                "scheduled",
                            )
                            .await;
                        }
                    }
                }
            });

            // Innate 自成长驱动器：启动时同步事件阈值，并按间隔对活跃项目跑 evolve（蒸馏 + 整理）作为兜底。
            let db_for_kb = db.clone();
            tauri::async_runtime::spawn(async move {
                let settings = commands::knowledge::load_knowledge_settings(&db_for_kb).await;
                knowledge::set_evolve_threshold(settings.capture_threshold);
                // 启动时把统一配置（蒸馏 LLM + embedding）载入 in-process Innate，
                // 确保即使只在 DB 改过配置、未手动保存，Innate 也用上最新模型。
                knowledge::refresh_kb_models(&db_for_kb).await;
                loop {
                    let hours = commands::knowledge::load_knowledge_settings(&db_for_kb)
                        .await
                        .evolve_interval_hours;
                    if hours == 0 {
                        // 定时器关闭：每小时复查一次配置是否被重新开启。
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        continue;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(hours as u64 * 3600)).await;
                    if let Ok(projects) = sqlx::query_as::<_, (String,)>(
                        "SELECT id FROM projects WHERE status='active'",
                    )
                    .fetch_all(&db_for_kb)
                    .await
                    {
                        for (pid,) in projects {
                            knowledge::kb_evolve(&pid).await;
                        }
                    }
                    // 通用（跨项目）库也定期整理。
                    knowledge::kb_evolve_shared().await;
                }
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
            commands::projects::create_local_project,
            commands::projects::clone_project_from_git,
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
            commands::change_requests::retry_change_request,
            commands::change_requests::delete_change_request,
            commands::conversations::list_conversations,
            commands::conversations::list_messages,
            commands::conversations::send_message,
            commands::conversations::import_attachment,
            commands::conversations::list_conversation_attachments,
            commands::conversations::open_attachment,
            commands::conversations::attachment_data_url,
            commands::conversations::create_group_conversation,
            commands::conversations::update_group_conversation,
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
            commands::knowledge::run_conversation_command,
            commands::knowledge::get_knowledge_settings,
            commands::knowledge::set_knowledge_settings,
            commands::knowledge::get_knowledge_embedding,
            commands::knowledge::set_knowledge_embedding,
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
            commands::settings::list_role_catalog,
            commands::settings::set_role_slot,
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
            commands::dev_server::get_dev_server_log,
            commands::cr_preview::get_cr_preview,
            commands::cr_preview::start_cr_preview,
            commands::cr_preview::stop_cr_preview,
            commands::cr_preview::launch_cr_app,
            commands::cr_preview::get_cr_preview_log,
            commands::materials::list_material_folders,
            commands::materials::create_material_folder,
            commands::materials::rename_material_folder,
            commands::materials::delete_material_folder,
            commands::materials::list_material_files,
            commands::materials::search_material_files,
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
            commands::security::list_security_audits,
            commands::deploy::list_deployments,
            commands::deploy::generate_deploy_script,
            commands::deploy::confirm_deploy,
            commands::prototype::list_prototype_prompts,
            commands::prototype::generate_prototype_prompt,
            commands::prototype::delete_prototype_prompt,
            commands::prototype::update_prototype_prompt,
            commands::artifacts::list_delivery_artifacts,
            commands::artifacts::import_delivery_artifact,
            commands::artifacts::update_delivery_artifact_meta,
            commands::artifacts::delete_delivery_artifact,
            commands::artifacts::delivery_artifact_data_url,
            commands::scan::run_proactive_scan,
            commands::grading::get_cr_grade,
            commands::grading::list_auto_pass_policy,
            commands::grading::get_auto_pass_enabled,
            commands::grading::set_auto_pass_enabled,
            commands::notify::list_notify_channels,
            commands::notify::create_notify_channel,
            commands::notify::update_notify_channel,
            commands::notify::delete_notify_channel,
            commands::notify::test_notify_channel,
            commands::widget::get_widget_snippet,
            commands::preview::mask_preview_data,
            commands::preview::provision_preview_container,
        ])
        .run(tauri::generate_context!())
        .expect("error running AutoForge");
}
