// ASR 模型管理命令
// 检查/下载/删除 SenseVoice 语音识别模型

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tracing::info;

const MODEL_DOWNLOAD_URL: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main";
const MODEL_FILE: &str = "model.onnx";
const TOKENS_FILE: &str = "tokens.txt";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AsrModelStatus {
    pub name: String,
    pub description: String,
    pub size_mb: u64,
    pub is_installed: bool,
    pub model_dir: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelDownloadProgress {
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub status: String,
}

/// 获取 ASR 模型目录
fn get_asr_models_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("models")
        .join("asr")
        .join("sense-voice")
}

/// 检查 ASR 模型是否已安装
#[tauri::command]
pub async fn check_asr_model(app: AppHandle) -> Result<AsrModelStatus, String> {
    let model_dir = get_asr_models_dir(&app);
    let has_model = model_dir.join(MODEL_FILE).exists();
    let has_tokens = model_dir.join(TOKENS_FILE).exists();

    Ok(AsrModelStatus {
        name: "SenseVoice Small".to_string(),
        description: "阿里通义实验室语音识别模型，支持中/英/日/韩/粤语".to_string(),
        size_mb: 900,
        is_installed: has_model && has_tokens,
        model_dir: model_dir.to_string_lossy().to_string(),
    })
}

/// 下载 ASR 模型
#[tauri::command]
pub async fn download_asr_model(app: AppHandle) -> Result<(), String> {
    let model_dir = get_asr_models_dir(&app);
    std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建目录失败: {}", e))?;

    let files = vec![
        (format!("{}/{}", MODEL_DOWNLOAD_URL, MODEL_FILE), model_dir.join(MODEL_FILE)),
        (format!("{}/{}", MODEL_DOWNLOAD_URL, TOKENS_FILE), model_dir.join(TOKENS_FILE)),
    ];

    let client = reqwest::Client::new();

    for (url, dest_path) in files {
        let file_name = dest_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string();

        // 如果文件已存在则跳过
        if dest_path.exists() {
            info!("[ASR] 文件已存在，跳过: {}", file_name);
            let _ = app.emit("model-download-progress", ModelDownloadProgress {
                file_name: file_name.clone(),
                downloaded_bytes: 0,
                total_bytes: 0,
                progress: 1.0,
                status: "completed".to_string(),
            });
            continue;
        }

        let _ = app.emit("model-download-progress", ModelDownloadProgress {
            file_name: file_name.clone(),
            downloaded_bytes: 0,
            total_bytes: 0,
            progress: 0.0,
            status: "downloading".to_string(),
        });

        let response = client.get(&url).send().await.map_err(|e| format!("下载请求失败: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("下载失败: HTTP {}", response.status()));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = tokio::fs::File::create(&dest_path).await.map_err(|e| format!("创建文件失败: {}", e))?;

        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("下载错误: {}", e))?;
            file.write_all(&chunk).await.map_err(|e| format!("写入失败: {}", e))?;

            downloaded += chunk.len() as u64;
            let progress = if total_size > 0 { downloaded as f32 / total_size as f32 } else { 0.0 };

            if downloaded % (100 * 1024) < chunk.len() as u64 || downloaded == total_size {
                let _ = app.emit("model-download-progress", ModelDownloadProgress {
                    file_name: file_name.clone(),
                    downloaded_bytes: downloaded,
                    total_bytes: total_size,
                    progress,
                    status: "downloading".to_string(),
                });
            }
        }

        let _ = app.emit("model-download-progress", ModelDownloadProgress {
            file_name: file_name.clone(),
            downloaded_bytes: total_size,
            total_bytes: total_size,
            progress: 1.0,
            status: "completed".to_string(),
        });

        info!("[ASR] 下载完成: {}", file_name);
    }

    Ok(())
}
