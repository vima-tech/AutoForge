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
            let backups = data_dir.join("backups").to_string_lossy().to_string();
            let opendesign = data_dir.join("opendesign").to_string_lossy().to_string();
            state::init_worktrees_base(worktrees);
            state::init_attachments_base(attachments);
            state::init_materials_base(materials);
            state::init_kb_base(kb);
            state::init_backups_base(backups);
            state::init_opendesign_base(opendesign);

            // 凭据加密：注入主密钥兜底文件路径并预热（确定 keychain/file 后端），
            // 必须在任何加解密与迁移之前。
            let master_key_file = data_dir.join("master.key").to_string_lossy().to_string();
            core::secrets::init_secrets(master_key_file);
            core::secrets::warm_up();

            let db = tauri::async_runtime::block_on(async {
                db::init(&db_path).await.expect("db init failed")
            });

            // 一次性把库内残留明文密钥就地加密（幂等，失败不阻断启动）。
            tauri::async_runtime::block_on(async {
                match core::secrets::migrate_plaintext_secrets(&db).await {
                    Ok(n) if n > 0 => println!("[secrets] 已加密迁移 {} 个明文密钥字段", n),
                    Ok(_) => {}
                    Err(e) => eprintln!("[secrets] 明文密钥迁移失败: {}", e),
                }
            });
            // 一次性为已有项目补全仓库内身份锚 .autoforge/project.json（幂等、非破坏，
            // 不动 DB；使旧项目也能在「删除后重新添加同一仓库」时挂回历史数据）。
            tauri::async_runtime::block_on(async {
                commands::projects::backfill_project_identities(&db).await;
            });

            let (max_slots, pause_threshold, queue_strategy) =
                tauri::async_runtime::block_on(commands::system::load_concurrency_settings(&db))
                    .expect("load concurrency settings failed");

            let concurrency =
                core::concurrency::ConcurrencyManager::new(max_slots, pause_threshold);
            concurrency.update_config(None, None, Some(queue_strategy));

            // 合并门构建池 + cgroup CPU 预算：按配置初始化。构建池全平台；CPU 预算仅
            // Linux 且 pct>0 时尝试，失败优雅降级（见 core::cpubudget）。
            let (build_slots, cpu_budget_pct) = tauri::async_runtime::block_on(async {
                (
                    commands::system::load_build_slots(&db).await,
                    commands::system::load_cpu_budget_pct(&db).await,
                )
            });
            state::init_build_pool(build_slots);
            core::cpubudget::init(cpu_budget_pct);

            let job_tx = tasks::runner::start(db.clone(), app_handle.clone(), concurrency.clone());

            let webhook_handle = std::sync::Arc::new(tokio::sync::Mutex::new(None));
            let autosupply_running =
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

            app.manage(AppState {
                db: db.clone(),
                job_tx: job_tx.clone(),
                concurrency,
                dev_servers: std::sync::Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                webhook_handle: webhook_handle.clone(),
                asr_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                autosupply_running: autosupply_running.clone(),
            });

            // 启动恢复：上次进程退出（崩溃/重启）时在途的代码实现任务，其内存轮询任务已
            // 随进程消失，但 CR 仍停在 pending_execution/executing 且无人再去抢槽位——批量
            // 合并腾空槽位也救不回。这里在任何 driver 任务产生前重排它们，使流水线自愈。
            let db_for_requeue = db.clone();
            let tx_for_requeue = job_tx.clone();
            tauri::async_runtime::spawn(async move {
                // 先回收上次崩溃残留的孤儿 agent 进程组（在旧 worktree 里还在烧 CPU 的
                // claude + 其子进程），再重排执行任务（重排会 fork 全新 worktree）。
                core::reaper::reap_orphans_under(&state::worktrees_base());
                tasks::runner::requeue_orphaned_executions(&db_for_requeue, &tx_for_requeue).await;
                // 同样救回卡在 pending_analysis 的孤儿需求：要么进程中途退出，要么旧版
                // 用稳定 analysis:<id> 重新分析时被已 completed 的 job 行去重而从未派发。
                tasks::runner::requeue_orphaned_analyses(&db_for_requeue, &tx_for_requeue).await;
                // 救回卡在 pending_merge 的孤儿合并：review_2/自动合并门/解冲突回落已置态并入队，
                // 但 Merge 驱动任务随进程消失。Merge 幂等（git merge --squash 重跑空操作），可安全重排。
                tasks::runner::requeue_orphaned_merges(&db_for_requeue, &tx_for_requeue).await;
                // 救回卡在 reverting 的孤儿撤销：git revert 不幂等，绝不自动重跑——回滚到稳定态
                // merged，由人确认 dev 后手动重试。
                tasks::runner::recover_orphaned_reverts(&db_for_requeue).await;
                // 关闭卡在 running 的孤儿会议室任务：交互式、有副作用（发消息/扣费/写文件），
                // 不自动重跑，标 failed 让用户重新发指令。
                commands::orchestration::fail_orphaned_conversation_tasks(&db_for_requeue).await;
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

            // 工厂自喂料调度（design 阶段 C）：按 autosupply.interval_min 对活跃项目跑
            // 扫描 + proposer，产物全部进 triage 池（永不自动进流水线）。默认关闭。
            let db_for_supply = db.clone();
            let tx_for_supply = job_tx.clone();
            let app_for_supply = app_handle.clone();
            let running_for_supply = autosupply_running.clone();
            tauri::async_runtime::spawn(async move {
                use tasks::autosupply;
                loop {
                    let cfg = autosupply::AutosupplyConfig::load(&db_for_supply).await;
                    if !cfg.enabled {
                        // 关闭时每分钟复查一次是否被重新开启（不睡满整个间隔）。
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        continue;
                    }
                    // 按「距上次实际运行已过多久」计算到下次触发的剩余时间，而非每次重启
                    // 都重新睡满一个完整间隔——这样 dev 热重载/频繁重启不再清零计时器。
                    let interval_secs = cfg.interval_min.max(5) * 60;
                    let wait_secs = match autosupply::last_run_unix(&db_for_supply).await {
                        Some(last) => {
                            let elapsed = (autosupply::now_unix() - last).max(0);
                            (interval_secs - elapsed).max(0)
                        }
                        None => 0, // 从未运行过 → 尽快补跑
                    };
                    // 即便已到期，也给启动留 20s 缓冲，避免开机瞬间与初始化抢资源。
                    let secs = if wait_secs == 0 { 20 } else { wait_secs as u64 };
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    // 醒来后重载配置（睡眠期间可能被关闭或改了间隔）。
                    let cfg = autosupply::AutosupplyConfig::load(&db_for_supply).await;
                    if cfg.enabled {
                        // run_cycle 内部会在成功运行后写入 last_run_at。
                        let _ = autosupply::run_cycle(
                            &db_for_supply,
                            &tx_for_supply,
                            &app_for_supply,
                            &cfg,
                            &running_for_supply,
                        )
                        .await;
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
                    if cfg.webhook_enabled {
                        let port = cfg.webhook_port as u16;
                        let db_clone = db_for_wh.clone();
                        let app_clone = app_for_wh.clone();
                        let handle = tokio::spawn(async move {
                            if let Err(e) =
                                intake::webhook::start(port, db_clone, job_tx.clone(), app_clone)
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

                // Linux/WebKitGTK 默认拒绝 getUserMedia，导致语音录入报 NotAllowedError。
                // 开启 media-stream 并自动放行麦克风/摄像头权限请求（本地桌面应用，可信）。
                // macOS/Windows 的 webview 默认即放行，无需处理。
                #[cfg(target_os = "linux")]
                {
                    use webkit2gtk::glib::object::ObjectExt;
                    use webkit2gtk::{
                        PermissionRequestExt, SettingsExt, UserMediaPermissionRequest, WebViewExt,
                    };
                    let _ = win.with_webview(|webview| {
                        let wv = webview.inner();
                        if let Some(settings) = WebViewExt::settings(&wv) {
                            settings.set_enable_media_stream(true);
                        }
                        // 仅放行麦克风/摄像头（UserMedia）请求，其余权限交回默认处理。
                        wv.connect_permission_request(|_, req| {
                            if req.is::<UserMediaPermissionRequest>() {
                                req.allow();
                                true
                            } else {
                                false
                            }
                        });
                    });
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
            commands::projects::set_default_project,
            commands::projects::delete_project,
            commands::projects::restore_project,
            commands::projects::purge_project,
            commands::projects::list_archived_projects,
            commands::issues::list_issues,
            commands::issues::list_issues_page,
            commands::issues::list_issue_statuses,
            commands::issues::list_issues_by_statuses,
            commands::issues::list_issue_titles,
            commands::issues::get_issue,
            commands::issues::get_issue_analysis,
            commands::issues::submit_issue,
            commands::issues::retry_analysis,
            commands::issues::reanalyze_with_feedback,
            commands::issues::update_issue_acceptance,
            commands::issues::import_issue_attachment,
            commands::issues::list_issue_attachments,
            commands::issues::issue_attachment_data_url,
            commands::issues::open_issue_attachment,
            commands::issues::delete_issue_attachment,
            commands::issues::list_cr_test_runs,
            commands::intake::get_intake_config,
            commands::intake::update_intake_config,
            commands::intake::get_webhook_status,
            commands::intake::sync_github_issues,
            commands::intake::bulk_import_issues,
            commands::intake::bulk_import_file,
            commands::intake::export_bulk_template,
            commands::intake::submit_from_artifact,
            commands::intake::decide_issue_draft,
            commands::intake::list_triage_issues,
            commands::intake::refine_triage,
            commands::intake::discard_triage,
            commands::intake::reject_issues,
            commands::intake::run_proposer,
            commands::intake::run_autosupply_now,
            commands::intake::autosupply_is_running,
            commands::settings::get_autosupply_settings,
            commands::settings::set_autosupply_settings,
            commands::settings::get_autonomy_level,
            commands::settings::set_autonomy_level,
            commands::change_requests::list_change_requests,
            commands::change_requests::list_change_requests_page,
            commands::change_requests::get_change_request_by_issue,
            commands::change_requests::get_change_request,
            commands::change_requests::get_default_merge_message,
            commands::change_requests::get_worktree_session,
            commands::change_requests::get_code_diff,
            commands::change_summary::generate_change_summary,
            commands::change_requests::get_merge_conflict,
            commands::change_requests::retry_merge,
            commands::change_requests::ai_resolve_merge_conflict,
            commands::change_requests::revert_change_request,
            commands::conflicts::get_conflict_detail,
            commands::conflicts::resolve_conflict_manually,
            commands::conflicts::open_conflict_workspace,
            commands::change_requests::review_1,
            commands::change_requests::review_1_batch,
            commands::change_requests::review_1_merge,
            commands::change_requests::split_change_request,
            commands::change_requests::get_change_request_issues,
            commands::requirement_merge::list_merge_candidates,
            commands::change_requests::review_2,
            commands::change_requests::review_2_batch,
            commands::change_requests::retry_change_request,
            commands::change_requests::restore_change_request,
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
            commands::conversation_archives::archive_conversation,
            commands::conversation_archives::list_conversation_archives,
            commands::conversation_archives::get_conversation_archive,
            commands::conversation_archives::search_conversation_archives,
            commands::conversation_archives::delete_conversation_archive,
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
            commands::orchestration::draft_coding_brief,
            commands::orchestration::draft_coding_brief_detailed,
            commands::orchestration::start_conversation_coding,
            commands::orchestration::list_conversation_tasks,
            commands::orchestration::compress_conversation_context,
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
            commands::settings::get_web_search_settings,
            commands::settings::set_web_search_settings,
            commands::settings::get_operator_profile,
            commands::settings::set_operator_profile,
            commands::notifications::list_notifications,
            commands::notifications::unread_notification_count,
            commands::notifications::mark_notification_read,
            commands::notifications::mark_all_notifications_read,
            commands::settings::get_asr_settings,
            commands::settings::set_asr_settings,
            commands::asr::transcribe_recording_segment,
            commands::asr::transcribe_recording_file,
            commands::asr::asr_realtime_start,
            commands::asr::asr_realtime_feed,
            commands::asr::asr_realtime_stop,
            commands::meetings::analyze_meeting,
            commands::meetings::save_meeting_doc,
            commands::settings::list_builtin_tools,
            commands::trace::list_llm_traces,
            commands::trace::get_llm_trace,
            commands::trace::list_trace_agent_names,
            commands::trace::clear_llm_traces,
            commands::agent_outputs::list_agent_outputs,
            commands::agent_outputs::get_agent_output,
            commands::agent_outputs::list_agent_output_roles,
            commands::agent_outputs::agent_output_field_health,
            commands::agent_outputs::clear_agent_outputs,
            commands::settings::secret_backend_status,
            commands::backup::export_config,
            commands::backup::import_config,
            commands::backup::reveal_backup,
            commands::mcp::list_mcp_servers,
            commands::mcp::create_mcp_server,
            commands::mcp::update_mcp_server,
            commands::mcp::delete_mcp_server,
            commands::mcp::test_mcp_connection,
            commands::mcp::discover_code_intel_map,
            commands::settings::list_agents,
            commands::settings::create_agent,
            commands::settings::update_agent,
            commands::settings::delete_agent,
            commands::settings::set_agent_forge_role,
            commands::settings::list_role_catalog,
            commands::settings::set_role_slot,
            commands::system::system_health,
            commands::system::system_resources,
            commands::system::check_claude_auth,
            commands::code_agents::list_code_agents,
            commands::code_agents::upsert_code_agent,
            commands::code_agents::delete_code_agent,
            commands::code_agents::set_default_code_agent,
            commands::code_agents::set_project_code_agent,
            commands::code_agents::check_code_agent_auth,
            commands::code_agents::list_code_agent_runs,
            commands::code_agents::get_code_agent_run,
            commands::code_agents::get_running_code_agent_log,
            commands::code_agent_skills::list_code_agent_skills,
            commands::code_agent_skills::upsert_code_agent_skill,
            commands::code_agent_skills::delete_code_agent_skill,
            commands::system::pipeline_stats,
            commands::system::get_badge_counts,
            commands::system::update_concurrency_config,
            commands::system::get_concurrency_config,
            commands::system::list_preview_environments,
            commands::system::list_test_sessions,
            commands::system::list_scan_findings,
            commands::system::list_admin_decisions,
            commands::system::list_job_failures,
            commands::self_update::self_update_status,
            commands::self_update::self_update_pull,
            commands::self_update::self_update_pending,
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
            commands::cr_preview::list_local_branches,
            commands::cr_preview::start_branch_preview,
            commands::cr_preview::list_branch_previews,
            commands::cr_preview::stop_branch_preview,
            commands::cr_preview::get_branch_preview_log,
            commands::cr_preview::start_preview_log_tail,
            commands::cr_preview::stop_preview_log_tail,
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
            commands::specs::scan_spec_files,
            commands::specs::get_spec_content,
            commands::specs::set_spec_injection,
            commands::blueprint::generate_project_blueprint,
            commands::blueprint::apply_project_blueprint,
            commands::blueprint::enqueue_blueprint_tasks,
            commands::security::list_security_audits,
            commands::deploy::list_deployments,
            commands::deploy::generate_deploy_script,
            commands::deploy::update_deploy_script,
            commands::deploy::delete_deployment,
            commands::deploy::confirm_deploy,
            commands::prototype::list_prototype_prompts,
            commands::prototype::generate_prototype_prompt,
            commands::prototype::delete_prototype_prompt,
            commands::prototype::update_prototype_prompt,
            commands::prototype::get_opendesign_settings,
            commands::prototype::set_opendesign_settings,
            commands::prototype::launch_opendesign,
            commands::prototype::get_opendesign_log,
            commands::artifacts::list_delivery_artifacts,
            commands::artifacts::import_delivery_artifact,
            commands::artifacts::update_delivery_artifact_meta,
            commands::artifacts::rename_delivery_artifact,
            commands::artifacts::delete_delivery_artifact,
            commands::artifacts::delivery_artifact_data_url,
            commands::artifacts::reveal_delivery_artifact,
            commands::scan::run_proactive_scan,
            commands::run_config::ai_generate_run_config,
            commands::grading::get_cr_grade,
            commands::grading::list_auto_pass_policy,
            commands::grading::get_auto_pass_enabled,
            commands::grading::set_auto_pass_enabled,
            commands::grading::get_auto_conflict_resolve_enabled,
            commands::grading::set_auto_conflict_resolve_enabled,
            commands::grading::get_parallel_premerge_enabled,
            commands::grading::set_parallel_premerge_enabled,
            commands::grading::get_custom_merge_message_enabled,
            commands::grading::set_custom_merge_message_enabled,
            commands::notify::list_notify_channels,
            commands::notify::create_notify_channel,
            commands::notify::update_notify_channel,
            commands::notify::delete_notify_channel,
            commands::notify::test_notify_channel,
            commands::notify::clawbot_start_login,
            commands::notify::clawbot_poll_login,
            commands::widget::get_widget_snippet,
            commands::widget::list_widget_tokens,
            commands::widget::create_widget_token,
            commands::widget::get_project_webhook_token,
            commands::widget::regenerate_project_webhook_token,
            commands::widget::set_widget_token_enabled,
            commands::widget::delete_widget_token,
            commands::preview::mask_preview_data,
            commands::preview::provision_preview_container,
        ])
        .build(tauri::generate_context!())
        .expect("error building AutoForge")
        .run(|_app, event| {
            // 退出时回收所有在途 code agent 进程组，杜绝退出/重启后 claude（及其
            // ripgrep/构建子进程）变成孤儿继续烧 CPU。
            if let tauri::RunEvent::Exit = event {
                core::reaper::kill_all();
            }
        });
}
