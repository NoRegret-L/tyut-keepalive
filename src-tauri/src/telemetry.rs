//! Structured, credential-free events and observed connection statistics.
//! A single journal feeds the dashboard and notification policy.
use crate::state::ConnectionState;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

// Disk budget: two 1 MiB files. UI IPC returns at most 200 rows.
const FILE_BUDGET: u64 = 1024 * 1024;
const ROW_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Initializing, NeedSetup, CredentialError, Checking, LoggingIn,
    Online, Offline, InvalidCredentials, AuthConfirmed, ConnectionLost,
    ConnectionRestored, CredentialsChanged, Paused, Suspended,
    SystemSuspended, SystemResumed, NetworkChanged,
}
impl From<ConnectionState> for Kind {
    fn from(state: ConnectionState) -> Self {
        match state {
            ConnectionState::Initializing => Self::Initializing,
            ConnectionState::NeedSetup => Self::NeedSetup,
            ConnectionState::CredentialError => Self::CredentialError,
            ConnectionState::Checking => Self::Checking,
            ConnectionState::LoggingIn => Self::LoggingIn,
            ConnectionState::Online => Self::Online,
            ConnectionState::Offline => Self::Offline,
            ConnectionState::InvalidCredentials => Self::InvalidCredentials,
            ConnectionState::Paused => Self::Paused,
            ConnectionState::Suspended => Self::Suspended,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Daily {
    date: String,
    auth_confirmed: u64,
    interruptions: u64,
}
impl Daily {
    fn roll(&mut self, date: &str) {
        if self.date != date { *self = Self { date: date.into(), ..Default::default() }; }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    time: String,
    kind: Kind,
    daily: Daily,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    daily: Daily,
    observed_online_seconds: Option<u64>,
    notifications_enabled: bool,
    persistence_ok: bool,
    rows: Vec<Row>,
}
pub(crate) struct Journal {
    dir: PathBuf,
    rows: VecDeque<Row>,
    daily: Daily,
    last_observation: Option<bool>,
    online_since: Option<Instant>,
    invalid_notified: bool,
    storage_notified: bool,
    enabled: bool,
    persistence_ok: bool,
    last_kind: Option<Kind>,
}
pub(crate) type Shared = Mutex<Journal>;

fn today() -> String { chrono::Local::now().format("%Y-%m-%d").to_string() }
fn private_open(path: &Path, append: bool) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).append(append).truncate(!append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn append_row(dir: &Path, row: &Row) -> std::io::Result<()> {
    let current = dir.join("events.jsonl");
    let backup = dir.join("events.previous.jsonl");
    let mut bytes = serde_json::to_vec(row).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let length = match fs::metadata(&current) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if bytes.len() as u64 > FILE_BUDGET {
        return Err(std::io::Error::other("event exceeds disk budget"));
    }
    // Reserve one byte for a possible separator after a torn final write.
    let mut rotated = false;
    if length.saturating_add(bytes.len() as u64).saturating_add(1) > FILE_BUDGET {
        match fs::remove_file(&backup) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        fs::rename(&current, &backup)?;
        rotated = true;
    }
    let needs_separator = if length > 0 && !rotated {
        let mut reader = fs::File::open(&current)?;
        reader.seek(SeekFrom::End(-1))?;
        let mut last = [0_u8];
        reader.read_exact(&mut last)?;
        last[0] != b'\n'
    } else { false };
    let mut file = private_open(&current, true)?;
    if needs_separator { file.write_all(b"\n")?; }
    file.write_all(&bytes)?;
    file.sync_data()
}
impl Journal {
    fn open(dir: PathBuf) -> Self {
        let mut journal = Self {
            dir, rows: VecDeque::new(), daily: Daily::default(),
            last_observation: None, online_since: None, invalid_notified: false,
            storage_notified: false, enabled: false, persistence_ok: true, last_kind: None,
        };
        if fs::create_dir_all(&journal.dir).is_err() { journal.persistence_ok = false; }
        // Load oldest first, and recover the daily aggregate from the newest valid event.
        for name in ["events.previous.jsonl", "events.jsonl"] {
            let path = journal.dir.join(name);
            match fs::metadata(&path) {
                Ok(meta) if meta.len() <= FILE_BUDGET => {
                    match fs::read_to_string(&path) {
                        Ok(data) => for line in data.lines() {
                            match serde_json::from_str::<Row>(line) {
                                Ok(row) => { journal.daily = row.daily.clone(); journal.push(row); }
                                Err(_) => journal.persistence_ok = false,
                            }
                        },
                        Err(_) => journal.persistence_ok = false,
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                _ => journal.persistence_ok = false,
            }
        }
        journal.daily.roll(&today());
        journal.enabled = fs::read_to_string(journal.dir.join("notifications")).ok().as_deref() == Some("true");
        journal
    }
    fn push(&mut self, row: Row) {
        self.rows.push_front(row);
        self.rows.truncate(ROW_LIMIT);
    }
    fn record(&mut self, kind: Kind) {
        self.daily.roll(&today());
        let row = Row { time: chrono::Local::now().to_rfc3339(), kind, daily: self.daily.clone() };
        if append_row(&self.dir, &row).is_err() {
            // Sticky for this process: later success cannot restore missing history.
            self.persistence_ok = false;
        }
        self.push(row);
    }
    fn reset_observation(&mut self) {
        self.last_observation = None;
        self.online_since = None;
        self.invalid_notified = false;
        self.storage_notified = false;
        self.record(Kind::CredentialsChanged);
    }
    // Count only accepted network observations, never transient Checking/LoggingIn states.
    fn observe(&mut self, online: bool, authenticated: bool) -> Option<Kind> {
        self.daily.roll(&today());
        let edge = match (self.last_observation, online) {
            (Some(true), false) => {
                self.daily.interruptions = self.daily.interruptions.saturating_add(1);
                Some(Kind::ConnectionLost)
            }
            (Some(false), true) => Some(Kind::ConnectionRestored),
            _ => None,
        };
        if online {
            if self.online_since.is_none() { self.online_since = Some(Instant::now()); }
            self.invalid_notified = false;
        } else {
            self.online_since = None;
        }
        self.last_observation = Some(online);
        if authenticated {
            self.daily.auth_confirmed = self.daily.auth_confirmed.saturating_add(1);
            self.record(Kind::AuthConfirmed);
        }
        if let Some(kind) = edge { self.record(kind); }
        edge
    }
    fn state(&mut self, state: ConnectionState, observation: Option<bool>, authenticated: bool) -> Option<Kind> {
        let mut notification = observation.and_then(|value| self.observe(value, authenticated));
        // An unclassified transport error is unknown, not a proven outage.
        // End the observed-duration span without fabricating an outage edge.
        if state == ConnectionState::Offline && observation.is_none() {
            self.online_since = None;
        }
        let kind = Kind::from(state);
        if self.last_kind != Some(kind) { self.record(kind); self.last_kind = Some(kind); }
        if state == ConnectionState::InvalidCredentials && !self.invalid_notified {
            self.invalid_notified = true;
            notification = Some(Kind::InvalidCredentials);
        }
        if state == ConnectionState::CredentialError && !self.storage_notified {
            self.storage_notified = true;
            notification = Some(Kind::CredentialError);
        }
        if matches!(state, ConnectionState::NeedSetup | ConnectionState::CredentialError | ConnectionState::InvalidCredentials | ConnectionState::Paused | ConnectionState::Suspended) {
            self.last_observation = None;
            self.online_since = None;
        }
        notification.filter(|_| self.enabled)
    }
    fn snapshot(&mut self) -> Snapshot {
        self.daily.roll(&today());
        Snapshot {
            daily: self.daily.clone(),
            observed_online_seconds: self.online_since.map(|start| start.elapsed().as_secs()),
            notifications_enabled: self.enabled, persistence_ok: self.persistence_ok,
            rows: self.rows.iter().cloned().collect(),
        }
    }
    fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let mut file = private_open(&self.dir.join("notifications"), false)
            .map_err(|_| "无法保存通知设置".to_string())?;
        file.write_all(if enabled { b"true" } else { b"false" })
            .and_then(|_| file.sync_data()).map_err(|_| "无法保存通知设置".to_string())?;
        self.enabled = enabled;
        Ok(enabled)
    }
}
pub(crate) fn init(app: &AppHandle) -> anyhow::Result<()> {
    let dir = app.path().app_data_dir()?.join("telemetry");
    app.manage(Mutex::new(Journal::open(dir)));
    Ok(())
}
pub(crate) fn snapshot(state: &Shared) -> Result<Snapshot, String> {
    Ok(state.lock().map_err(|_| "运行记录暂不可用".to_string())?.snapshot())
}
pub(crate) fn set_enabled(state: &Shared, enabled: bool) -> Result<bool, String> {
    state.lock().map_err(|_| "通知设置暂不可用".to_string())?.set_enabled(enabled)
}
pub(crate) fn credentials_changed(app: &AppHandle) {
    if let Some(state) = app.try_state::<Shared>() {
        if let Ok(mut journal) = state.lock() { journal.reset_observation(); }
    }
}
pub(crate) fn context_changed(app: &AppHandle, kind: Kind) {
    if let Some(shared) = app.try_state::<Shared>() {
        if let Ok(mut journal) = shared.lock() {
            journal.last_observation = None;
            journal.online_since = None;
            journal.record(kind);
        }
    }
}
pub(crate) fn on_state(app: &AppHandle, state: ConnectionState, observation: Option<bool>, authenticated: bool) {
    let Some(shared) = app.try_state::<Shared>() else { return; };
    let notification = if let Ok(mut journal) = shared.lock() {
        journal.state(state, observation, authenticated)
    } else { None };
    let _ = app.emit("telemetry-changed", ());
    if let Some(kind) = notification {
        let body = match kind {
            Kind::ConnectionLost => "检测到连接中断，后台将尝试恢复。",
            Kind::ConnectionRestored => "已重新检测到网络连接。",
            Kind::InvalidCredentials => "校园网密码错误，请更新凭据。",
            Kind::CredentialError => "系统凭据存储异常，请检查应用设置。",
            _ => return,
        };
        // Notification failure never interrupts keepalive, and no credential data is sent.
        if app.notification().builder().title(crate::APP_DISPLAY_NAME).body(body).show().is_err() {
            crate::log("系统通知发送失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("tyut-journal-{}-{}", std::process::id(), rand::random::<u64>())))
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    #[test]
    fn transitions_count_once_and_survive_restart() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        j.observe(true, true);
        j.state(ConnectionState::Checking, None, false);
        j.observe(true, false);
        assert_eq!(j.daily.interruptions, 0);
        assert_eq!(j.observe(false, false), Some(Kind::ConnectionLost));
        j.observe(false, false);
        assert_eq!(j.observe(true, false), Some(Kind::ConnectionRestored));
        assert_eq!(j.daily.interruptions, 1);
        assert_eq!(j.daily.auth_confirmed, 1);
        let recovered = Journal::open(temp.0.clone());
        assert_eq!(recovered.daily, j.daily);
        assert!(recovered.online_since.is_none());
    }
    #[test]
    fn pause_and_suspend_end_observation_without_outage() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        for state in [ConnectionState::Paused, ConnectionState::Suspended] {
            j.observe(true, false);
            j.state(state, None, false);
            assert!(j.online_since.is_none());
            assert!(j.last_observation.is_none());
            j.observe(false, false);
            assert_eq!(j.daily.interruptions, 0);
        }
    }
    #[test]
    fn credentials_change_does_not_count_as_outage() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        j.observe(true, false);
        j.reset_observation();
        j.observe(false, false);
        assert_eq!(j.daily.interruptions, 0);
    }
    #[test]
    fn daily_counter_rolls_and_notifications_deduplicate() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        j.daily.auth_confirmed = 9;
        j.daily.roll("different-day");
        assert_eq!(j.daily.auth_confirmed, 0);
        j.set_enabled(true).unwrap();
        assert_eq!(j.state(ConnectionState::InvalidCredentials, None, false), Some(Kind::InvalidCredentials));
        j.state(ConnectionState::LoggingIn, None, false);
        assert_eq!(j.state(ConnectionState::InvalidCredentials, None, false), None);
        assert!(Journal::open(temp.0.clone()).enabled);
    }
    #[test]
    fn unclassified_failure_ends_duration_without_counting_outage() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        j.observe(true, false);
        j.state(ConnectionState::Offline, None, false);
        assert!(j.online_since.is_none());
        assert_eq!(j.daily.interruptions, 0);
        j.observe(true, false);
        assert!(j.online_since.is_some());
        assert_eq!(j.daily.interruptions, 0);
    }
    #[test]
    fn append_after_torn_line_preserves_new_events() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        fs::write(temp.0.join("events.jsonl"), b"{\"incomplete\":").unwrap();
        j.observe(true, true);
        let recovered = Journal::open(temp.0.clone());
        assert_eq!(recovered.daily.auth_confirmed, 1);
        assert!(!recovered.persistence_ok);
    }
    #[test]
    fn rotation_retains_latest_aggregate() {
        let temp = Temp::new();
        let mut j = Journal::open(temp.0.clone());
        fs::write(temp.0.join("events.jsonl"), vec![b' '; FILE_BUDGET as usize]).unwrap();
        j.observe(true, true);
        assert!(temp.0.join("events.previous.jsonl").exists());
        assert!(fs::metadata(temp.0.join("events.jsonl")).unwrap().len() < FILE_BUDGET);
        assert_eq!(Journal::open(temp.0.clone()).daily.auth_confirmed, 1);
    }
}
