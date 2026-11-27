use tauri::{AppHandle, Manager, State};

use crate::{core::config, state::{Event, Scheduler}, SharedStatus};

#[tauri::command]
pub(crate) fn get_status(status: State<'_, SharedStatus>) -> Result<serde_json::Value, String> {
    let guard = status.lock().map_err(|_| "状态锁被占用".to_string())?;
    serde_json::to_value(guard.clone()).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn get_telemetry(state: State<'_, crate::telemetry::Shared>) -> Result<crate::telemetry::Snapshot, String> {
    crate::telemetry::snapshot(&state)
}
#[tauri::command]
pub(crate) fn set_notifications(state: State<'_, crate::telemetry::Shared>, enabled: bool) -> Result<bool, String> {
    crate::telemetry::set_enabled(&state, enabled)
}
#[tauri::command]
pub(crate) fn reconnect(scheduler: State<'_, Scheduler>) -> Result<(), String> {
    scheduler.send(Event::Reconnect)
}

async fn wait_reply(rx: std::sync::mpsc::Receiver<Result<(), String>>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv().map_err(|_| "后台调度器已停止".to_string())?
    }).await.map_err(|_| "后台任务已停止".to_string())?
}

#[tauri::command]
pub(crate) async fn set_paused(scheduler: State<'_, Scheduler>, paused: bool) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    scheduler.send(Event::SetPaused(paused, Some(tx)))?;
    wait_reply(rx).await
}
#[tauri::command]
pub(crate) async fn reset_credentials(scheduler: State<'_, Scheduler>) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    scheduler.send(Event::Clear(tx))?;
    wait_reply(rx).await
}

#[tauri::command]
pub(crate) async fn setup_credentials(
    scheduler: State<'_, Scheduler>,
    account: String,
    password: String,
) -> Result<(), String> {
    if account.trim().is_empty() || password.is_empty() {
        return Err("账号和密码不能为空".into());
    }
    let value = config::Credentials {
        account: config::xor_encode(account.trim()),
        password: config::xor_encode(&password),
        saved_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    scheduler.send(Event::Save(value, tx))?;
    wait_reply(rx).await
}

#[tauri::command]
pub(crate) fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|error| error.to_string())?;
    } else {
        autostart.disable().map_err(|error| error.to_string())?;
    }
    autostart.is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn is_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub(crate) fn show_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub(crate) fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if crate::tray::is_tray_available() {
            let _ = window.hide();
        } else {
            let _ = window.minimize();
        }
    }
}

