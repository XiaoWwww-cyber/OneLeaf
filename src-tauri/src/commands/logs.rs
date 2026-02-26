use crate::utils::log_dir::LOG_DIR;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogInfo {
    pub log_dir: String,
    pub files: Vec<LogFileInfo>,
    pub total_size_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFileInfo {
    pub name: String,
    pub size_bytes: u64,
}

/// 获取日志目录信息
#[tauri::command]
pub async fn get_log_info() -> Result<LogInfo, String> {
    let log_dir = LOG_DIR.lock().clone().ok_or("日志目录未初始化")?;
    let dir_str = log_dir.to_string_lossy().to_string();

    let mut files = Vec::new();
    let mut total_size: u64 = 0;

    if log_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&log_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        let size = meta.len();
                        total_size += size;
                        files.push(LogFileInfo {
                            name: entry.file_name().to_string_lossy().to_string(),
                            size_bytes: size,
                        });
                    }
                }
            }
        }
    }

    // 按名称倒序（最新的在前）
    files.sort_by(|a, b| b.name.cmp(&a.name));

    Ok(LogInfo {
        log_dir: dir_str,
        files,
        total_size_mb: total_size as f64 / (1024.0 * 1024.0),
    })
}

/// 清除所有日志文件
#[tauri::command]
pub async fn clear_logs() -> Result<(), String> {
    let log_dir = LOG_DIR.lock().clone().ok_or("日志目录未初始化")?;

    if log_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&log_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    info!("所有日志文件已清除");
    Ok(())
}
