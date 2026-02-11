// 视频处理命令
// 支持视频上传、音频提取、ASR 语音识别

use crate::core::sidecar_manager::ASR_GPU_PORT;
use crate::utils::paths::get_temp_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: String,
    pub path: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TranscriptResult {
    pub text: String,
}

/// 上传视频（返回视频信息）
#[tauri::command]
pub async fn upload_video(_app: AppHandle, path: String) -> Result<VideoInfo, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err("文件不存在".to_string());
    }

    let name = p
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();

    Ok(VideoInfo { id, path, name })
}

/// 转写视频 - 提取音频后调用 ASR GPU 服务
#[tauri::command]
pub async fn transcribe_video(
    app: AppHandle,
    video_path: String,
) -> Result<TranscriptResult, String> {
    let temp_dir = get_temp_dir(&app);
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let video_id = uuid::Uuid::new_v4().to_string();
    let audio_path = temp_dir.join(format!("{}.wav", video_id));

    // 1. 提取音频 - 使用内嵌的 ffmpeg 或系统 ffmpeg
    let ffmpeg_path = find_ffmpeg(&app);
    extract_audio_with_ffmpeg(&ffmpeg_path, &video_path, &audio_path)?;

    // 2. 调用 ASR GPU 服务进行转写
    let text = call_asr_service(&audio_path).await?;

    // 3. 清理临时音频文件
    let _ = fs::remove_file(&audio_path);

    Ok(TranscriptResult { text })
}

/// 查找 FFmpeg 路径
fn find_ffmpeg(app: &AppHandle) -> String {
    // 优先使用资源目录中的 ffmpeg
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidates = [
            resource_dir
                .join("resources")
                .join("ffmpeg")
                .join("ffmpeg.exe"),
            resource_dir.join("ffmpeg").join("ffmpeg.exe"),
        ];
        for c in &candidates {
            if c.exists() {
                return c.to_string_lossy().to_string();
            }
        }
    }
    // 回退到系统 PATH 中的 ffmpeg
    "ffmpeg".to_string()
}

/// 使用 FFmpeg 提取音频
fn extract_audio_with_ffmpeg(
    ffmpeg_path: &str,
    video_path: &str,
    audio_path: &Path,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new(ffmpeg_path);
    cmd.arg("-i")
        .arg(video_path)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-y")
        .arg(audio_path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd.output().map_err(|e| format!("FFmpeg 执行失败: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg 音频提取失败: {}", err));
    }

    Ok(())
}

/// 调用 ASR GPU Python 服务进行语音转写 (SSE Stream)
async fn call_asr_service(audio_path: &Path) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/transcribe", ASR_GPU_PORT);
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let request_body = serde_json::json!({
        "audio_path": audio_path_str,
        "use_gpu": true,
        "num_threads": 4
    });

    // 使用 reqwest 调用 ASR 服务
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(600))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            format!(
                "ASR 服务连接失败 ({}): {}。请确保 ASR 服务已启动且模型已下载。",
                url, e
            )
        })?;

    // 流式读取 SSE 响应
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 ASR 响应失败: {}", e))?;

    let mut final_text = String::new();
    let mut success = false;

    for line in body.lines() {
        if line.starts_with("data: ") {
            let json_str = &line[6..];
            if json_str.trim() == "[DONE]" {
                break;
            }

            if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(status) = data["status"].as_str() {
                    if status == "error" {
                        let msg = data["error"].as_str().unwrap_or("未知错误");
                        return Err(format!("ASR 转写失败: {}", msg));
                    }
                    if status == "success" {
                        final_text = data["text"].as_str().unwrap_or("").to_string();
                        success = true;
                    }
                }
            }
        }
    }

    if !success {
        return Err("ASR 服务未返回结果，请检查模型是否已下载".to_string());
    }

    Ok(final_text)
}
