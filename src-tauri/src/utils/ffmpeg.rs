use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FfmpegError {
    #[error("FFmpeg not found")]
    NotFound,
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("IO Error: {0}")]
    IoError(#[from] std::io::Error),
}

pub struct FfmpegWrapper;

impl FfmpegWrapper {
    pub fn new() -> Result<Self, FfmpegError> {
        // Simple check if ffmpeg is in path
        match Command::new("ffmpeg").arg("-version").output() {
            Ok(output) => {
                if output.status.success() {
                    Ok(Self)
                } else {
                    Err(FfmpegError::NotFound)
                }
            }
            Err(_) => Err(FfmpegError::NotFound),
        }
    }

    pub fn extract_audio(&self, video_path: &Path, output_path: &Path) -> Result<(), FfmpegError> {
        let output = Command::new("ffmpeg")
            .arg("-i")
            .arg(video_path)
            .arg("-vn") // No video
            .arg("-acodec")
            .arg("pcm_s16le") // Wav PCM
            .arg("-ar")
            .arg("16000") // 16kHz for ASR
            .arg("-ac")
            .arg("1") // Mono
            .arg("-y") // Overwrite
            .arg(output_path)
            .output()?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(FfmpegError::ExecutionFailed(err.to_string()));
        }

        Ok(())
    }
}
