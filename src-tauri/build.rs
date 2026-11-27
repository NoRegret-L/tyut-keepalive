fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "get_status",
                "get_telemetry",
                "set_notifications",
                "reconnect",
                "set_paused",
                "reset_credentials",
                "setup_credentials",
                "set_autostart",
                "is_autostart",
                "quit",
                "hide_window",
                "update_enabled",
                "check_update",
                "install_update",
            ]),
        ),
    ).expect("Tauri security manifest generation failed");
}
