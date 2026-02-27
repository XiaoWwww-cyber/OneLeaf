// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod ai;
pub mod commands;
pub mod core;
pub mod utils;

use tauri::Manager;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn open_settings(app: tauri::AppHandle) {
    core::tray::open_settings_window(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.get_webview_window("main")
                .and_then(|w| {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                    Some(())
                });
        }))
        .setup(|app| {
            // 初始化日志系统（终端 + 按天滚动文件）
            let log_dir = app.path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("logs");
            std::fs::create_dir_all(&log_dir).ok();

            // 保存日志目录路径到全局变量
            *utils::log_dir::LOG_DIR.lock() = Some(log_dir.clone());

            let file_appender = tracing_appender::rolling::daily(&log_dir, "oneleaf.log");
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

            // 泄漏 guard 使其在整个程序生命周期有效
            std::mem::forget(_guard);

            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;

            tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()))
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
                .with(tracing_subscriber::fmt::layer().with_writer(non_blocking).with_ansi(false))
                .init();

            tracing::info!("日志系统初始化完成，日志目录: {:?}", log_dir);

            // 初始化系统托盘
            if let Err(e) = core::tray::init_tray(app.handle()) {
                eprintln!("托盘初始化失败: {}", e);
            }

            // 初始化 ASR Sidecar 服务
            core::sidecar_manager::init_sidecar(app.handle());

            // 启动时读取持久化的 AI 配置
            commands::ai::init_ai_settings(app.handle());

            // 监听菜单事件
            let app_handle = app.handle().clone();
            app.on_menu_event(move |_app, event| {
                match event.id().as_ref() {
                    "show" => {
                        core::tray::show_main_window(&app_handle);
                    }
                    "settings" => {
                        core::tray::open_settings_window(&app_handle);
                    }
                    "quit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::ai::init_knowledge_base,
            commands::ai::add_document_to_kb,
            commands::ai::search_knowledge_base,
            commands::ai::chat_with_ai,
            commands::ai::upload_file_to_ai,
            commands::ai::list_documents,
            commands::ai::delete_document,
            commands::ai::get_document_content,
            commands::ai::open_document_file,
            commands::ai::get_ai_settings,
            commands::ai::update_ai_settings,
            commands::ai::check_lm_studio,
            commands::ai::clear_all_cache,
            commands::ai::get_embedding_model_status,
            commands::ai::download_embedding_model,
            commands::ai::save_conversation,
            commands::ai::load_conversations,
            commands::ai::delete_conversation_record,
            commands::ai::save_message,
            commands::ai::load_messages,
            commands::asr::check_asr_model,
            commands::asr::download_asr_model,
            commands::video::upload_video,
            commands::video::transcribe_video,
            commands::logs::get_log_info,
            commands::logs::clear_logs,
            open_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
