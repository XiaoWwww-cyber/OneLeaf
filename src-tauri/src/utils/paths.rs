use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn get_app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn get_temp_dir(app: &AppHandle) -> PathBuf {
    get_app_data_dir(app).join("temp")
}

pub fn get_models_dir(app: &AppHandle) -> PathBuf {
    get_app_data_dir(app).join("models")
}
