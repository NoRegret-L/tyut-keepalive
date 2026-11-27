mod commands;
mod core;
mod state;
mod tray;
mod telemetry;
mod updates;
use crate::core::{config, dns};
use tauri::Manager;

pub(crate) type SharedStatus = state::SharedStatus;

pub(crate) const TEST_BUILD: bool = cfg!(feature = "test-build");
pub(crate) const APP_IDENTIFIER: &str = if TEST_BUILD {
    "com.tyut.keepalive.test"
} else {
    "com.tyut.keepalive"
};
pub(crate) const APP_DISPLAY_NAME: &str = if TEST_BUILD {
    "TYUT Keepalive Test"
} else {
    "TYUT Keepalive"
};

fn identity_matches(identifier: &str) -> bool {
    identifier == APP_IDENTIFIER
}

pub(crate) const PORTAL_URL: &str = "http://219.226.127.250";
pub(crate) const LOGIN_HOST: &str = "https://drcom.tyut.edu.cn:804";
pub(crate) const LOGIN_DOMAIN: &str = "drcom.tyut.edu.cn";
pub(crate) const TARGET: &str = "223.5.5.5";
pub(crate) const REFERER_URL: &str = "https://drcom.tyut.edu.cn:804/eportal/portal/index";
pub(crate) const USER_AGENT_VALUE: &str = "Mozilla/5.0 Chrome/146.0.0.0 Edg/146.0.0.0";
pub(crate) const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36 Edg/146.0.0.0";

pub(crate) fn log(message: impl AsRef<str>) {
    println!(
        "[{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        message.as_ref()
    );
}

#[cfg(test)]
mod tests {
    use crate::state::Status;

    #[test]
    fn build_identity_matches_only_its_own_namespace() {
        assert!(super::identity_matches(super::APP_IDENTIFIER));
        if super::TEST_BUILD {
            assert!(!super::identity_matches("com.tyut.keepalive"));
        } else {
            assert!(!super::identity_matches("com.tyut.keepalive.test"));
        }
    }
    #[test]
    fn status_serializes_camel_case() {
        let status = Status {
            state: crate::state::ConnectionState::Online,
            account: "d****03".into(),
            last_check: "2026-11-26 15:00:00".into(),
            last_login: "尚未登录".into(),
            message: "网络正常".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"lastCheck\""));
        assert!(json.contains("\"lastLogin\""));
    }


}

pub fn run() {
    let from_autostart = std::env::args().any(|arg| arg == "--from-autostart");
    let shared: SharedStatus = std::sync::Arc::new(std::sync::Mutex::new(state::Status::default()));
    // Fail closed before any plugin accesses local data or registers an instance.
    // A test feature must never run with the production Tauri identifier, or vice versa.
    let context = tauri::generate_context!();
    assert!(identity_matches(&context.config().identifier), "build identity/config mismatch");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .manage(updates::UpdateState::default())
        .plugin(tauri_plugin_power_monitor::init());
    // Do not even register the updater in a test build, regardless of future
    // changes to a public key in the base configuration.
    let app = if TEST_BUILD { app } else {
        app.plugin(tauri_plugin_updater::Builder::new().build())
    };
    let app = if TEST_BUILD {
        app.plugin(tauri_plugin_autostart::Builder::new()
            .app_name(APP_DISPLAY_NAME)
            .args(["--from-autostart"])
            .build())
    } else {
        // Preserve the existing production autostart name and migration.
        app.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--from-autostart"]),
        ))
    };
    let app = app.manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_telemetry,
            commands::set_notifications,
            commands::reconnect,
            commands::set_paused,
            commands::reset_credentials,
            commands::setup_credentials,
            commands::set_autostart,
            commands::is_autostart,
            commands::quit,
            updates::update_enabled,
            updates::check_update,
            updates::install_update,
            commands::hide_window,
        ])
        .setup(move |app| {
            telemetry::init(app.handle())?;
            if tray::create_tray(app.handle()).is_err() {
                log("系统托盘初始化失败，使用窗口降级");
            }
            state::start_worker(app.handle().clone(), shared)?;
            if !from_autostart || !tray::is_tray_available() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                }
            }
            // [设计意图/根因] -> 旧版本自启项没有 --from-autostart 参数
            // [理论先验/数学假设] -> 插件 enable 会覆盖同名自启项并写入当前参数
            // [机制闭环解释] -> 已开启自启的用户升级后自动获得静默启动，无需手动开关
            {
                use tauri_plugin_autostart::ManagerExt;
                if app.autolaunch().is_enabled().unwrap_or(false) {
                    let _ = app.autolaunch().enable();
                }
            }
            Ok(())
        })
        .build(context)
        .expect("Tauri 应用构建失败");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } = event
        {
            api.prevent_close();
            if label == "main" {
                if let Some(window) = app_handle.get_webview_window(&label) {
                    if tray::is_tray_available() {
                        let _ = window.hide();
                    } else {
                        let _ = window.minimize();
                    }
                }
            }
        }
    });
}
