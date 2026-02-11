// 系统托盘功能

use tauri::{
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    menu::{Menu, MenuItem, PredefinedMenuItem},
    Manager, AppHandle,
    WebviewWindowBuilder, WebviewUrl,
};
use std::sync::atomic::{AtomicBool, Ordering};

static TRAY_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// 初始化系统托盘
pub fn init_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if TRAY_INITIALIZED.swap(true, Ordering::SeqCst) {
        return Err("托盘已初始化".into());
    }

    let app_handle = app.clone();

    let menu = create_tray_menu(app)?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("OneLeaf - 智能知识库")
        .on_tray_icon_event(move |_tray, event| {
            handle_tray_event(&app_handle, event);
        })
        .build(app)?;

    Ok(())
}

/// 创建托盘菜单
fn create_tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[
        &show_item,
        &settings_item,
        &separator,
        &quit_item,
    ])?;

    Ok(menu)
}

/// 处理托盘事件
fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    match event {
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
            show_main_window(app);
        }
        _ => {}
    }
}

/// 显示主窗口
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.unminimize();
    }
}

/// 打开设置窗口
pub fn open_settings_window(app: &AppHandle) {
    // 如果已存在则激活
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    // 创建新窗口
    let _window = WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("/settings".into()),
    )
    .title("OneLeaf 设置")
    .inner_size(700.0, 600.0)
    .min_inner_size(500.0, 400.0)
    .resizable(true)
    .center()
    .build();
}
