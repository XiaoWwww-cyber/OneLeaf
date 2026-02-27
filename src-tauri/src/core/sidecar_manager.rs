// ASR Python 服务管理器
// 启动嵌入式 Python 运行 ASR GPU 服务

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use tauri::{AppHandle, Manager};
use tracing::{error, info};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// ASR GPU 服务端口
pub const ASR_GPU_PORT: u16 = 38081;

/// Sidecar 状态
pub struct SidecarState {
    pub asr_gpu_pid: Mutex<Option<u32>>,
}

/// 初始化 ASR Sidecar 服务
pub fn init_sidecar(app: &AppHandle) {
    let resource_dir = app
        .path()
        .resource_dir()
        .expect("获取资源目录失败");

    // 查找嵌入式 Python
    let python_candidates = [
        resource_dir.join("resources").join("python-embed").join("python.exe"),
        resource_dir.join("python-embed").join("python.exe"),
    ];

    let python_path = match python_candidates.iter().find(|p| p.exists()).cloned() {
        Some(p) => p,
        None => {
            info!("[Sidecar] 未找到嵌入式 Python，ASR 服务不可用");
            for p in &python_candidates {
                info!("[Sidecar]   搜索路径: {:?}", p);
            }
            return;
        }
    };

    // 查找 ASR 脚本
    let asr_script_candidates = [
        resource_dir.join("resources").join("asr-service").join("asr_gpu_server.py"),
        resource_dir.join("asr-service").join("asr_gpu_server.py"),
    ];

    let asr_script = match asr_script_candidates.iter().find(|p| p.exists()).cloned() {
        Some(p) => p,
        None => {
            info!("[Sidecar] 未找到 ASR 脚本，ASR 服务不可用");
            return;
        }
    };

    info!("[ASR-GPU] Python: {:?}", python_path);
    info!("[ASR-GPU] Script: {:?}", asr_script);

    // 清理可能残留的 ASR 进程
    info!("[Sidecar] 扫描残留的 ASR 进程...");
    let mut sys = sysinfo::System::new_all();
    for (pid, process) in sys.processes() {
        let name_str = format!("{:?}", process.name()).to_lowercase();
        if name_str.contains("python") {
            let mut should_kill = false;

            let exe_str = format!("{:?}", process.exe()).to_lowercase();
            if exe_str.contains("python-embed") {
                should_kill = true;
            }

            if !should_kill {
                let cmd_str = format!("{:?}", process.cmd()).to_lowercase();
                if cmd_str.contains("asr_gpu_server.py") || cmd_str.contains("python-embed") {
                    should_kill = true;
                }
            }

            if should_kill {
                info!("[Sidecar] 发现残留进程: PID {}, 准备终止", pid);
                process.kill();
            }
        }
    }

    // 启动 ASR GPU 服务
    let mut cmd = Command::new(&python_path);
    cmd.arg("-u") // 强制无缓冲输出
        .arg(&asr_script)
        .env("ASR_GPU_PORT", ASR_GPU_PORT.to_string())
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .current_dir(&resource_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            info!("[ASR-GPU] 服务已启动, PID: {}", pid);
            info!("[ASR-GPU] 服务地址: http://127.0.0.1:{}", ASR_GPU_PORT);

            // 读取 stdout
            if let Some(stdout) = child.stdout.take() {
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            info!("[ASR-GPU Python] {}", line);
                        }
                    }
                });
            }

            // 读取 stderr
            if let Some(stderr) = child.stderr.take() {
                thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            error!("[ASR-GPU Error] {}", line);
                        }
                    }
                });
            }

            app.manage(SidecarState {
                asr_gpu_pid: Mutex::new(Some(pid)),
            });
        }
        Err(e) => {
            error!("[ASR-GPU] 启动失败: {}", e);
        }
    }
}

/// 终止 ASR Sidecar 服务及其子进程
pub fn kill_sidecar(app: &AppHandle) {
    let pid_to_kill = {
        let state = match app.try_state::<SidecarState>() {
            Some(s) => s,
            None => return,
        };

        let x = if let Ok(mut pid_guard) = state.asr_gpu_pid.lock() {
            let pid = *pid_guard;
            *pid_guard = None;
            pid
        } else {
            None
        };
        x
    };

    if let Some(pid) = pid_to_kill {
        info!("[ASR-GPU] 准备终止进程树, PID: {}", pid);

        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status();
        }

        info!("[ASR-GPU] 进程已请求终止");
    }
}
