use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use crate::state::{ConnectionState, Event, Scheduler, Status};

static TRAY_AVAILABLE: AtomicBool = AtomicBool::new(false);
struct TrayUi { status: MenuItem<tauri::Wry>, reconnect: MenuItem<tauri::Wry>, pause: MenuItem<tauri::Wry>, resume: MenuItem<tauri::Wry> }

pub(crate) fn is_tray_available() -> bool { TRAY_AVAILABLE.load(Ordering::Relaxed) }

fn label(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Initializing => "正在初始化",
        ConnectionState::NeedSetup => "需要设置凭据",
        ConnectionState::CredentialError => "凭据存储异常",
        ConnectionState::Checking => "正在检查",
        ConnectionState::LoggingIn => "正在认证",
        ConnectionState::Online => "校园网已连接",
        ConnectionState::Offline => "等待自动重试",
        ConnectionState::InvalidCredentials => "密码错误",
        ConnectionState::Paused => "保活已暂停",
        ConnectionState::Suspended => "休眠中",
    }
}
// Tiny local status icon: color plus distinct geometry; no external assets.
fn icon(state: ConnectionState) -> Image<'static> {
    let (color, symbol) = match state {
        ConnectionState::Online => ([22, 163, 74], 0),
        ConnectionState::Offline => ([217, 119, 6], 1),
        ConnectionState::CredentialError | ConnectionState::InvalidCredentials => ([220, 38, 38], 1),
        ConnectionState::Checking | ConnectionState::LoggingIn => ([37, 99, 235], 2),
        _ => ([100, 116, 139], 2),
    };
    let mut rgba = vec![0; 24 * 24 * 4];
    for y in 0_i32..24 {
        for x in 0_i32..24 {
            let dx = 2 * x - 23; let dy = 2 * y - 23;
            if dx * dx + dy * dy > 21 * 21 { continue; }
            let white = match symbol {
                0 => ((6..=11).contains(&x) && (y - x - 4).abs() <= 1) ||
                     ((10..=18).contains(&x) && (y + x - 26).abs() <= 1),
                1 => ((10..=13).contains(&x) && (6..=13).contains(&y)) ||
                     ((10..=13).contains(&x) && (16..=18).contains(&y)),
                _ => (6..=17).contains(&x) && (10..=13).contains(&y),
            };
            let at = ((y * 24 + x) * 4) as usize;
            rgba[at..at+3].copy_from_slice(if white { &[255,255,255] } else { &color });
            rgba[at+3] = 255;
        }
    }
    Image::new_owned(rgba, 24, 24)
}
fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show(); let _ = window.unminimize(); let _ = window.set_focus();
    }
}
pub(crate) fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "正在初始化", false, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", "显示界面", true, None::<&str>)?;
    let reconnect = MenuItem::with_id(app, "reconnect", "立即重连", false, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "应用设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "彻底退出", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "暂停保活", false, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "恢复保活", false, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &show_item, &reconnect, &pause, &resume, &settings, &quit])?;
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon(ConnectionState::Initializing))
        .tooltip(crate::APP_DISPLAY_NAME)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show(app),
            "settings" => { show(app); let _ = app.emit("open-settings", ()); }
            "reconnect" => if let Some(scheduler) = app.try_state::<Scheduler>() {
                let _ = scheduler.send(Event::Reconnect);
            },
            "pause" | "resume" => if let Some(scheduler) = app.try_state::<Scheduler>() {
                let _ = scheduler.send(Event::SetPaused(event.id.as_ref() == "pause", None));
            },
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show(tray.app_handle());
            }
        }).build(app);
    match tray {
        Ok(_) => {
            app.manage(TrayUi { status, reconnect, pause, resume });
            TRAY_AVAILABLE.store(true, Ordering::Relaxed);
        }
        Err(_) => {
            TRAY_AVAILABLE.store(false, Ordering::Relaxed);
            crate::log("系统托盘不可用，已启用窗口降级");
        }
    }
    Ok(())
}
pub(crate) fn update(app: &AppHandle, status: &Status) {
    let state = status.state;
    if !is_tray_available() { return; }
    if let Some(ui) = app.try_state::<TrayUi>() {
        let _ = ui.status.set_text(label(state));
        let _ = ui.reconnect.set_enabled(!status.paused && !status.sleeping && !matches!(state, ConnectionState::Initializing | ConnectionState::NeedSetup | ConnectionState::CredentialError | ConnectionState::LoggingIn));
        let _ = ui.pause.set_enabled(!status.paused);
        let _ = ui.resume.set_enabled(status.paused);
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_icon(Some(icon(state)));
        // Linux may not support tooltips; status menu is the fallback.
        let _ = tray.set_tooltip(Some(format!("{} · {}", crate::APP_DISPLAY_NAME, label(state))));
    }
}
