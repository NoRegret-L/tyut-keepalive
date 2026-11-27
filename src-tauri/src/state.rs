use anyhow::Result;
use serde::Serialize;
use std::{
    sync::{atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, RecvTimeoutError, Sender}, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Listener, Manager};
use crate::core::{config, credentials, portal};

const WATCH_INTERVAL: Duration = Duration::from_secs(600);
const RETRY_INITIAL: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConnectionState {
    #[default]
    Initializing,
    NeedSetup,
    CredentialError,
    Checking,
    LoggingIn,
    Online,
    Offline,
    InvalidCredentials,
    Paused,
    Suspended,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub(crate) state: ConnectionState,
    pub(crate) account: String,
    pub(crate) last_check: String,
    pub(crate) last_login: String,
    pub(crate) message: String,
    pub(crate) paused: bool,
    pub(crate) sleeping: bool,
    #[serde(serialize_with = "serialize_schedule")]
    pub(crate) schedule: Option<Schedule>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Schedule {
    deadline: Instant,
    total: Duration,
    task: Task,
}

impl Schedule {
    fn remaining_ms(&self, now: Instant) -> u64 {
        self.deadline.saturating_duration_since(now).as_millis() as u64
    }
}

fn serialize_schedule<S: serde::Serializer>(schedule: &Option<Schedule>, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Snapshot {
        kind: &'static str,
        remaining_ms: u64,
        total_ms: u64,
    }
    schedule.as_ref().map(|s| Snapshot {
        kind: match s.task { Task::Check => "check", Task::Retry => "retry", Task::Login => "login" },
        remaining_ms: s.remaining_ms(Instant::now()),
        total_ms: s.total.as_millis() as u64,
    }).serialize(serializer)
}


pub(crate) type SharedStatus = Arc<Mutex<Status>>;
pub(crate) type Reply = Sender<std::result::Result<(), String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Task {
    Check,
    Login,
    Retry,
}
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Checked(bool),
    Authenticated(portal::LoginOutcome),
    Failed,
}
pub(crate) enum Event {
    Reconnect,
    SetPaused(bool, Option<Reply>),
    Suspend,
    Wake,
    NetworkChanged,
    Save(config::Credentials, Reply),
    Clear(Reply),
    Finished { generation: u64, task: Task, outcome: Outcome },
}

pub(crate) struct Scheduler(Sender<Event>);
impl Scheduler {
    pub(crate) fn send(&self, event: Event) -> std::result::Result<(), String> {
        self.0.send(event).map_err(|_| "后台调度器不可用".into())
    }
}

// Orthogonal policy gates: a wake or topology change must never lift user pause
// or the credential-error latch. Explicit reconnect may retry a bad password.
#[derive(Default)]
struct Gate {
    paused: bool,
    sleeping: bool,
    invalid: bool,
}
impl Gate {
    fn automatic(&self, has_credentials: bool) -> bool {
        has_credentials && !self.paused && !self.sleeping && !self.invalid
    }
    fn manual(&self, has_credentials: bool) -> bool {
        has_credentials && !self.paused && !self.sleeping
    }
}
fn invalidate(generation: &mut u64, cancellation: &Option<Arc<AtomicBool>>) {
    if let Some(flag) = cancellation { flag.store(true, Ordering::Release); }
    *generation = generation.wrapping_add(1);
}
fn publish_gate(app: &AppHandle, shared: &SharedStatus, status: &mut Status,
                gate: &Gate, idle: ConnectionState, has_credentials: bool) {
    status.paused = gate.paused;
    status.sleeping = gate.sleeping;
    let (state, message) = if gate.paused {
        (ConnectionState::Paused, "自动保活已暂停，不会主动注销网络")
    } else if gate.sleeping {
        (ConnectionState::Suspended, "系统休眠，等待唤醒")
    } else if gate.invalid {
        (ConnectionState::InvalidCredentials, "密码错误，请更新凭据或手动重连")
    } else if !has_credentials && idle == ConnectionState::CredentialError {
        (ConnectionState::CredentialError, "凭据存储异常，请检查设置")
    } else if !has_credentials {
        (ConnectionState::NeedSetup, "请设置账号密码")
    } else {
        (ConnectionState::Checking, "等待重新检查网络")
    };
    publish(app, shared, status, state, message);
}
fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
fn publish(app: &AppHandle, shared: &SharedStatus, status: &mut Status,
           state: ConnectionState, message: impl Into<String>) {
    publish_observed(app, shared, status, state, message, None, false);
}
fn publish_observed(app: &AppHandle, shared: &SharedStatus, status: &mut Status,
                    state: ConnectionState, message: impl Into<String>, observation: Option<bool>, authenticated: bool) {
    status.state = state;
    status.message = message.into();
    if !matches!(state, ConnectionState::Online | ConnectionState::Offline) {
        status.schedule = None;
    }
    emit_status(app, shared, status);
    crate::telemetry::on_state(app, state, observation, authenticated);
    crate::tray::update(app, status);
}

fn emit_status(app: &AppHandle, shared: &SharedStatus, status: &Status) {
    if let Ok(mut guard) = shared.lock() {
        *guard = status.clone();
    }
    let _ = app.emit("status", status.clone());
}

fn due(task: Task, delay: Duration) -> Option<(Instant, Task)> {
    Some((Instant::now() + delay, task))
}
fn next_event(rx: &Receiver<Event>, deadline: Option<Instant>) -> std::result::Result<Event, RecvTimeoutError> {
    match deadline {
        Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
    }
}

fn current_result(generation: u64, current: u64, pending: bool) -> bool {
    generation == current && !pending
}

fn launch(tx: Sender<Event>, generation: u64, task: Task, value: config::Credentials, cancelled: Arc<AtomicBool>) {
    thread::spawn(move || {
        let outcome = (|| -> Result<Outcome> {
            anyhow::ensure!(!cancelled.load(Ordering::Acquire), "任务已取消");
            if matches!(task, Task::Check | Task::Retry) {
                let client = portal::client(None)?;
                let online = portal::check_network(&client);
                if task == Task::Check || online {
                    return Ok(Outcome::Checked(online));
                }
            }
            let result = portal::attempt_login(&value, None, &cancelled)?;
            Ok(match result {
                portal::LoginOutcome::Reachable => Outcome::Checked(true),
                other => Outcome::Authenticated(other),
            })
        })().unwrap_or(Outcome::Failed);
        // Network errors may include sensitive URLs. Only send a classified result.
        let _ = tx.send(Event::Finished { generation, task, outcome });
    });
}

pub(crate) fn start_worker(app: AppHandle, shared: SharedStatus) -> Result<()> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let legacy = dir.join("config.enc");
    let (tx, rx) = mpsc::channel();
    app.manage(Scheduler(tx.clone()));
    // Listen in Rust, independent of the WebView and its visibility.
    let suspend_tx = tx.clone();
    app.listen("power-monitor://suspend", move |_| { let _ = suspend_tx.send(Event::Suspend); });
    let wake_tx = tx.clone();
    app.listen("power-monitor://resume", move |_| { let _ = wake_tx.send(Event::Wake); });
    thread::spawn(move || {
        // The handle must live for the lifetime of the worker. No raw address
        // or interface metadata is copied to logs, IPC, or the network.
        let network_pending = Arc::new(AtomicBool::new(false));
        let callback_pending = network_pending.clone();
        let network_tx = tx.clone();
        let _watch = match netwatcher::watch_interfaces_with_callback(move |update| {
            if !update.is_initial && !callback_pending.swap(true, Ordering::AcqRel) {
                if network_tx.send(Event::NetworkChanged).is_err() {
                    callback_pending.store(false, Ordering::Release);
                }
            }
        }) {
            Ok(handle) => Some(handle),
            Err(_) => { crate::log("网络事件监听不可用，保留定时检查"); None }
        };
        let mut status = Status {
            account: "未配置".into(),
            last_login: "尚未登录".into(),
            ..Default::default()
        };
        let mut active = match credentials::load(&legacy) {
            Ok(value) => value,
            Err(error) => {
                publish(&app, &shared, &mut status, ConnectionState::CredentialError, error.to_string());
                None
            }
        };
        let mut gate = Gate::default();
        let mut idle = if status.state == ConnectionState::CredentialError {
            ConnectionState::CredentialError
        } else { ConnectionState::NeedSetup };
        let mut generation = 0_u64;
        let mut retry = RETRY_INITIAL;
        let mut running: Option<(u64, Task)> = None;
        let mut cancellation: Option<Arc<AtomicBool>> = None;
        let mut next = None;
        if let Some(value) = &active {
            status.account = config::masked_account(&value.account);
            next = due(Task::Check, Duration::ZERO);
            publish(&app, &shared, &mut status, ConnectionState::Checking, "准备检查网络");
        } else if status.state != ConnectionState::CredentialError {
            publish(&app, &shared, &mut status, ConnectionState::NeedSetup, "请设置账号密码");
        }
        loop {
            // No timer or disk polling while idle; only one network job may run.
            let deadline = if running.is_none() { next.map(|(at, _)| at) } else { None };
            match next_event(&rx, deadline) {
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    let Some((_, task)) = next.take() else { continue; };
                    // Last gate check at dispatch: no timer or recovery event may bypass pause.
                    if !gate.manual(active.is_some()) || (gate.invalid && task != Task::Login) { continue; }
                    let Some(value) = active.clone() else { continue; };
                    running = Some((generation, task));
                    status.last_check = now();
                    let (state, message) = if task == Task::Check {
                        (ConnectionState::Checking, "正在检查网络")
                    } else {
                        (ConnectionState::LoggingIn, "正在尝试登录")
                    };
                    publish(&app, &shared, &mut status, state, message);
                    let cancelled = Arc::new(AtomicBool::new(false));
                    cancellation = Some(cancelled.clone());
                    launch(tx.clone(), generation, task, value, cancelled);
                }
                Ok(Event::Reconnect) => {
                    if !gate.manual(active.is_some()) {
                        continue;
                    }
                    if running != Some((generation, Task::Login)) {
                        // Only explicit reconnect is allowed to retry rejected credentials.
                        invalidate(&mut generation, &cancellation);
                        next = due(Task::Login, Duration::ZERO);
                        status.schedule = None;
                        emit_status(&app, &shared, &status);
                    }
                }
                Ok(Event::SetPaused(paused, reply)) => {
                    if gate.paused != paused {
                        gate.paused = paused;
                        invalidate(&mut generation, &cancellation);
                        next = None;
                        if !paused && gate.automatic(active.is_some()) {
                            retry = RETRY_INITIAL;
                            next = due(Task::Check, Duration::ZERO);
                        }
                        publish_gate(&app, &shared, &mut status, &gate, idle, active.is_some());
                    }
                    if let Some(reply) = reply { let _ = reply.send(Ok(())); }
                }
                Ok(Event::Suspend) => {
                    if gate.sleeping { continue; }
                    gate.sleeping = true;
                    invalidate(&mut generation, &cancellation);
                    next = None;
                    crate::telemetry::context_changed(&app, crate::telemetry::Kind::SystemSuspended);
                    publish_gate(&app, &shared, &mut status, &gate, idle, active.is_some());
                }
                Ok(Event::Wake) => {
                    // Accept wake even when the platform missed the preceding suspend.
                    gate.sleeping = false;
                    invalidate(&mut generation, &cancellation);
                    next = None;
                    crate::telemetry::context_changed(&app, crate::telemetry::Kind::SystemResumed);
                    if gate.automatic(active.is_some()) {
                        retry = RETRY_INITIAL;
                        next = due(Task::Check, Duration::ZERO);
                    }
                    publish_gate(&app, &shared, &mut status, &gate, idle, active.is_some());
                }
                Ok(Event::NetworkChanged) => {
                    network_pending.store(false, Ordering::Release);
                    if !gate.automatic(active.is_some()) { continue; }
                    // Coalesce changes while a replacement check is already queued.
                    if matches!(next, Some((at, Task::Check)) if at <= Instant::now()) { continue; }
                    invalidate(&mut generation, &cancellation);
                    retry = RETRY_INITIAL;
                    next = due(Task::Check, Duration::ZERO);
                    crate::telemetry::context_changed(&app, crate::telemetry::Kind::NetworkChanged);
                    publish(&app, &shared, &mut status, ConnectionState::Checking, "网络接口变化，重新检查");
                }
                Ok(Event::Save(value, reply)) => {
                    // Invalidate results from prior credentials before touching storage.
                    invalidate(&mut generation, &cancellation);
                    gate.invalid = false;
                    crate::telemetry::credentials_changed(&app);
                    active = None;
                    next = None;
                    retry = RETRY_INITIAL;
                    status.account = "未配置".into();
                    status.last_login = "尚未登录".into();
                    status.last_check.clear();
                    let result = credentials::save(&legacy, &value).map_err(|error| error.to_string());
                    match &result {
                        Ok(()) => {
                            status.account = config::masked_account(&value.account);
                            active = Some(value);
                            idle = ConnectionState::NeedSetup;
                            if gate.automatic(active.is_some()) {
                                next = due(Task::Login, Duration::ZERO);
                                publish(&app, &shared, &mut status, ConnectionState::Checking, "凭据已保存，等待认证");
                            } else {
                                publish_gate(&app, &shared, &mut status, &gate, idle, true);
                            }
                        }
                        Err(_) => {
                            idle = ConnectionState::CredentialError;
                            publish_gate(&app, &shared, &mut status, &gate, idle, false);
                        }
                    }
                    let _ = reply.send(result);
                }
                Ok(Event::Clear(reply)) => {
                    invalidate(&mut generation, &cancellation);
                    gate.invalid = false;
                    crate::telemetry::credentials_changed(&app);
                    active = None;
                    next = None;
                    retry = RETRY_INITIAL;
                    status.account = "未配置".into();
                    status.last_login = "尚未登录".into();
                    status.last_check.clear();
                    let result = credentials::clear(&legacy).map_err(|error| error.to_string());
                    idle = if result.is_ok() { ConnectionState::NeedSetup } else { ConnectionState::CredentialError };
                    publish_gate(&app, &shared, &mut status, &gate, idle, false);
                    let _ = reply.send(result);
                }
                Ok(Event::Finished { generation: completed, task, outcome }) => {
                    if running != Some((completed, task)) { continue; }
                    running = None;
                    cancellation = None;
                    // A new credential or queued manual action supersedes an old result.
                    if !current_result(completed, generation, next.is_some()) || !gate.manual(active.is_some()) { continue; }
                    status.last_check = now();
                    match outcome {
                        Outcome::Checked(true) | Outcome::Authenticated(portal::LoginOutcome::Success) => {
                            gate.invalid = false;
                            if matches!(outcome, Outcome::Authenticated(_)) { status.last_login = now(); }
                            retry = RETRY_INITIAL;
                            next = due(Task::Check, WATCH_INTERVAL);
                            status.schedule = next.map(|(deadline, task)| Schedule { deadline, task, total: WATCH_INTERVAL });
                            publish_observed(&app, &shared, &mut status, ConnectionState::Online, "网络正常，10 分钟后复查", Some(true), matches!(outcome, Outcome::Authenticated(portal::LoginOutcome::Success)));
                        }
                        Outcome::Authenticated(portal::LoginOutcome::BadPassword) => {
                            gate.invalid = true;
                            next = None;
                            publish(&app, &shared, &mut status, ConnectionState::InvalidCredentials, "密码错误，自动重试已停止，请更新凭据");
                        }
                        other => {
                            // Failed explicit retry of a rejected password stays latched.
                            if gate.invalid {
                                publish_gate(&app, &shared, &mut status, &gate, idle, true);
                                continue;
                            }
                            if !matches!(other, Outcome::Checked(false)) {
                                retry = (retry * 2).min(RETRY_MAX);
                            }
                            next = due(Task::Retry, retry);
                            status.schedule = next.map(|(deadline, task)| Schedule { deadline, task, total: retry });
                            // A transport error alone is not proof of a lost network link.
                            let observation = match other {
                                Outcome::Checked(false) | Outcome::Authenticated(portal::LoginOutcome::Indeterminate) => Some(false),
                                _ => None,
                            };
                            publish_observed(&app, &shared, &mut status, ConnectionState::Offline,
                                format!("网络异常，{} 秒后重试", retry.as_secs()), observation, false);
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_uses_actual_deadline_and_saturates() {
        let start = Instant::now();
        let schedule = Schedule { deadline: start + Duration::from_secs(10), total: Duration::from_secs(10), task: Task::Retry };
        assert_eq!(schedule.remaining_ms(start + Duration::from_secs(3)), 7000);
        assert_eq!(schedule.remaining_ms(start + Duration::from_secs(12)), 0);
        let status = Status { schedule: Some(schedule), ..Default::default() };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["schedule"]["kind"], "retry");
        assert_eq!(value["schedule"]["totalMs"], 10000);
        assert!(value["schedule"]["remainingMs"].as_u64().unwrap() <= 10000);
        assert!(serde_json::to_value(Status::default()).unwrap()["schedule"].is_null());
    }

    #[test]
    fn command_interrupts_future_deadline() {
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Reconnect).unwrap_or_else(|_| panic!("send failed"));
        assert!(matches!(next_event(&rx, Some(Instant::now() + WATCH_INTERVAL)), Ok(Event::Reconnect)));
    }
    #[test]
    fn old_generation_and_superseded_result_are_rejected() {
        assert!(!current_result(1, 2, false));
        assert!(!current_result(2, 2, true));
        assert!(current_result(2, 2, false));
    }
    #[test]
    fn pause_sleep_and_invalid_credentials_are_independent_gates() {
        let mut gate = Gate::default();
        assert!(gate.automatic(true));
        assert!(!gate.automatic(false));
        gate.paused = true;
        gate.sleeping = true;
        gate.sleeping = false; // wake does not resume user pause
        assert!(!gate.automatic(true));
        assert!(!gate.manual(true));
        gate.invalid = true;
        gate.paused = false;
        assert!(!gate.automatic(true));
        assert!(gate.manual(true));
        gate.sleeping = true;
        assert!(!gate.manual(true));
    }
    #[test]
    fn invalidate_cancels_inflight_and_rejects_old_result() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut generation = 1;
        invalidate(&mut generation, &Some(flag.clone()));
        assert!(flag.load(Ordering::Acquire));
        assert!(!current_result(1, generation, false));
    }
    #[test]
    fn no_deadline_waits_for_command() {
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Reconnect).unwrap_or_else(|_| panic!("send failed"));
        assert!(matches!(next_event(&rx, None), Ok(Event::Reconnect)));
    }
}
