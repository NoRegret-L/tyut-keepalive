//! Only the compiled-in trust policy may select an update source and key.
//! The WebView receives metadata, not a general-purpose installer or URL API.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const ENDPOINT: &str = "https://github.com/NoRegret-L/tyut-keepalive/releases/latest/download/latest.json";
const DOWNLOAD_PREFIX: &str = "/NoRegret-L/tyut-keepalive/releases/download/";

#[derive(Default)]
pub(crate) struct UpdateState {
    busy: AtomicBool,
    pending: Mutex<Option<Update>>,
}
struct BusyGuard<'a>(&'a AtomicBool);
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}
impl UpdateState {
    fn enter(&self) -> Result<BusyGuard<'_>, String> {
        self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| BusyGuard(&self.busy))
            .map_err(|_| "更新操作正在执行".into())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateInfo {
    version: String,
    notes: String,
}
fn policy(config: &serde_json::Value) -> bool {
    config.get("pubkey").and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty())
        && config.get("endpoints") == Some(&serde_json::json!([ENDPOINT]))
        && config.get("requireSignedVersion").and_then(|v| v.as_bool()) == Some(true)
        && config.get("allowDowngrades").and_then(|v| v.as_bool()) == Some(false)
        && config.get("dangerousInsecureTransportProtocol").and_then(|v| v.as_bool()) == Some(false)
}
fn enabled(app: &AppHandle) -> bool {
    !crate::TEST_BUILD && app.config().identifier == crate::APP_IDENTIFIER
        && app.config().plugins.0.get("updater").is_some_and(policy)
}
fn trusted_download(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("github.com")
        && url.port().is_none() && url.username().is_empty() && url.password().is_none()
        && url.path().starts_with(DOWNLOAD_PREFIX)
}
#[tauri::command]
pub(crate) fn update_enabled(app: AppHandle) -> bool { enabled(&app) }

#[tauri::command]
pub(crate) async fn check_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<Option<UpdateInfo>, String> {
    let _busy = state.enter()?;
    *state.pending.lock().map_err(|_| "更新状态不可用".to_string())? = None;
    if !enabled(&app) { return Err("签名更新尚未配置可信公钥或安全策略".into()); }
    let updater = app.updater().map_err(|_| "更新组件初始化失败".to_string())?;
    let update = updater.check().await.map_err(|_| "检查更新失败：请确认网络和发布清单".to_string())?;
    let Some(update) = update else { return Ok(None); };
    if !trusted_download(&update.download_url) || update.signature.trim().is_empty() {
        return Err("更新地址或签名不符合信任策略".into());
    }
    let info = UpdateInfo {
        version: update.version.clone(),
        notes: update.body.as_deref().unwrap_or("").chars().take(4000).collect(),
    };
    *state.pending.lock().map_err(|_| "更新状态不可用".to_string())? = Some(update);
    Ok(Some(info))
}

#[tauri::command]
pub(crate) async fn install_update(app: AppHandle, state: State<'_, UpdateState>, version: String) -> Result<(), String> {
    let _busy = state.enter()?;
    if !enabled(&app) { return Err("签名更新不可用".into()); }
    let update = state.pending.lock().map_err(|_| "更新状态不可用".to_string())?.take()
        .ok_or_else(|| "请先检查更新".to_string())?;
    if update.version != version || !trusted_download(&update.download_url) {
        return Err("更新信息已改变，请重新检查".into());
    }
    // Official updater verifies the package signature AND its signed version
    // before installation. Do not pass unverified bytes to install().
    update.download_and_install(|_, _| {}, || {}).await
        .map_err(|_| "更新失败：下载、签名验证或安装未完成，请重新检查更新".to_string())?;
    // Windows' installer manages process exit/restart. On Unix restart only
    // after a successful verified installation.
    #[cfg(not(target_os = "windows"))]
    app.restart();
    #[cfg(target_os = "windows")]
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_without_key_and_rejects_insecure_policy() {
        let mut p = serde_json::json!({
            "pubkey": "", "endpoints": [ENDPOINT], "requireSignedVersion": true,
            "allowDowngrades": false, "dangerousInsecureTransportProtocol": false
        });
        assert!(!policy(&p));
        p["pubkey"] = serde_json::json!("configured-for-test");
        assert!(policy(&p));
        p["allowDowngrades"] = serde_json::json!(true);
        assert!(!policy(&p));
        p["allowDowngrades"] = serde_json::json!(false);
        p["requireSignedVersion"] = serde_json::json!(false);
        assert!(!policy(&p));
        p["requireSignedVersion"] = serde_json::json!(true);
        p["endpoints"] = serde_json::json!(["https://example.org/latest.json"]);
        assert!(!policy(&p));
    }
    #[test]
    fn download_url_must_be_exact_repository_https() {
        let valid = format!("https://github.com{DOWNLOAD_PREFIX}v0.4.0/app.msi");
        assert!(trusted_download(&reqwest::Url::parse(&valid).unwrap()));
        for bad in [
            "http://github.com/NoRegret-L/tyut-keepalive/releases/download/v0.4.0/app.msi",
            "https://github.com.evil.example/NoRegret-L/tyut-keepalive/releases/download/v0.4.0/app.msi",
            "https://github.com/Code-famer-horse/tyut-keepalive/releases/download/v0.4.0/app.msi",
            "https://user@github.com/NoRegret-L/tyut-keepalive/releases/download/v0.4.0/app.msi",
        ] { assert!(!trusted_download(&reqwest::Url::parse(bad).unwrap())); }
    }
    #[test]
    fn update_operations_are_serialized() {
        let state = UpdateState::default();
        let guard = state.enter().unwrap();
        assert!(state.enter().is_err());
        drop(guard);
        assert!(state.enter().is_ok());
    }
}
