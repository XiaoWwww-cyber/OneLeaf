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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 初始化系统托盘
            if let Err(e) = core::tray::init_tray(app.handle()) {
                eprintln!("托盘初始化失败: {}", e);
            }

            // 初始化 ASR Sidecar 服务
            core::sidecar_manager::init_sidecar(app.handle());

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
            commands::ai::list_documents,
            commands::ai::delete_document,
            commands::asr::check_asr_model,
            commands::asr::download_asr_model,
            commands::video::upload_video,
            commands::video::transcribe_video
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
