// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod ai;
pub mod commands;
pub mod core;
pub mod utils;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 初始化 ASR Sidecar 服务
            core::sidecar_manager::init_sidecar(app.handle());
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
            commands::video::upload_video,
            commands::video::transcribe_video
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

