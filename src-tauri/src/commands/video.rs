// 视频处理命令
// 支持视频上传、音频提取、ASR 语音识别

use crate::core::sidecar_manager::ASR_GPU_PORT;
use crate::utils::paths::get_temp_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Manager};
use tracing::{error, info, warn};
use futures_util::StreamExt;

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
    info!("[Video] 开始转写视频: {}", video_path);
    let temp_dir = get_temp_dir(&app);
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let video_id = uuid::Uuid::new_v4().to_string();
    let audio_path = temp_dir.join(format!("{}.wav", video_id));
    info!("[Video] 临时音频路径: {:?}", audio_path);

    // 1. 提取音频 - 使用内嵌的 ffmpeg 或系统 ffmpeg
    let ffmpeg_path = find_ffmpeg(&app);
    info!("[Video] 使用 FFmpeg 路径: {}", ffmpeg_path);
    extract_audio_with_ffmpeg(&ffmpeg_path, &video_path, &audio_path)?;
    info!("[Video] 音频提取完成");

    // 2. 调用 ASR GPU 服务进行转写
    info!("[Video] 调用 ASR 服务...");
    let text = call_asr_service(&audio_path).await?;
    info!("[Video] ASR 转写成功，字数: {}", text.len());

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

    info!("[Video] 执行 FFmpeg 命令: {:?}", cmd);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd.output().map_err(|e| {
        error!("[Video] FFmpeg 启动失败: {}", e);
        format!("FFmpeg 执行失败: {}", e)
    })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        error!("[Video] FFmpeg 提取音频失败: {}", err);
        return Err(format!("FFmpeg 音频提取失败: {}", err));
    }

    info!("[Video] FFmpeg 提取音频成功");

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
            let err_msg = format!(
                "ASR 服务连接失败 ({}): {}。请确保 ASR 服务已启动且模型已下载。",
                url, e
            );
            error!("[Video] {}", err_msg);
            err_msg
        })?;

    info!("[Video] ASR 服务响应状态: {}", response.status());

    // 检查是否为 SSE 流
    let is_sse = response.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false);

    if !is_sse {
        warn!("[Video] ASR 服务未返回 event-stream，尝试读取全文内容");
        // 如果不是 SSE，回退到普通文本读取
        let text = response.text().await.map_err(|e| format!("读取非流式响应失败: {}", e))?;
        return Ok(text);
    }

    // 以字节流方式逐块读取 SSE 响应
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut final_text = String::new();
    let mut success = false;
    let mut done = false;
    let mut processed_chunks = 0;

    while let Some(chunk_result) = stream.next().await {
        if done { break; }
        
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                error!("[Video] 读取 ASR 数据块失败 (已处理 {} 个块): {}", processed_chunks, e);
                return Err(format!("网络读取中断: {}。如果此问题持续出现，请尝试重启应用以释放占用的进程或资源。", e));
            }
        };
        processed_chunks += 1;
        let chunk_str = String::from_utf8_lossy(&chunk);
        buffer.push_str(&chunk_str);

        // 按行解析
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            if line.starts_with("data: ") {
                let json_str = &line[6..];
                if json_str.trim() == "[DONE]" {
                    done = true;
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
                            done = true; // 拿到结果，可以提前退出了
                            break;
                        }
                    }
                }
            }
        }
    }

    if !success {
        return Err("ASR 服务未返回成功标志，请检查 GPU 是否可用。".to_string());
    }

    Ok(final_text)
}
