use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::path::PathBuf;

/// 全局日志目录路径（在 setup 阶段写入）
pub static LOG_DIR: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));
