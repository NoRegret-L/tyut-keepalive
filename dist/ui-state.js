export const THEMES = Object.freeze(["system", "light", "dark"]);
export function normalizeTheme(value) { return THEMES.includes(value) ? value : "system"; }
export function readTheme(storage) {
  try { return normalizeTheme(storage.getItem("tyut.theme")); } catch { return "system"; }
}
export function saveTheme(storage, theme) {
  try { storage.setItem("tyut.theme", normalizeTheme(theme)); return true; } catch { return false; }
}
export const STATES = Object.freeze({
  initializing: ["正在初始化", "neutral"],
  "need-setup": ["需要设置凭据", "neutral"],
  "credential-error": ["凭据存储异常", "danger"],
  checking: ["正在检查网络", "active"],
  "logging-in": ["正在认证", "active"],
  online: ["校园网已连接", "success"],
  offline: ["等待自动重试", "warning"],
  "invalid-credentials": ["密码错误", "danger"],
  paused: ["自动保活已暂停", "neutral"],
  suspended: ["系统休眠，等待唤醒", "neutral"],
});
export function describeState(state) {
  return Object.hasOwn(STATES, state) ? STATES[state] : ["未知状态", "neutral"];
}
// Bounded in-memory UI history, never persisted and never includes credentials.
export const HISTORY_LIMIT = 100;
export function addHistory(history, status, receivedAt) {
  const [label, tone] = describeState(status.state);
  const entry = { state: status.state, label, tone, message: String(status.message || ""), receivedAt };
  const prior = history[0];
  if (prior && prior.state === entry.state && prior.message === entry.message) return history;
  return [entry, ...history].slice(0, HISTORY_LIMIT);
}

export function keepaliveControls(status) {
  const paused = Boolean(status?.paused);
  const sleeping = Boolean(status?.sleeping);
  const blocked = ["initializing", "need-setup", "credential-error", "logging-in"].includes(status?.state);
  return { paused, reconnect: Boolean(status) && !paused && !sleeping && !blocked };
}

export const DIAGNOSTIC_EVENT_KINDS = Object.freeze([
  ...Object.keys(STATES), "auth-confirmed", "connection-lost", "connection-restored",
  "credentials-changed", "system-suspended", "system-resumed", "network-changed"
]);
export function filterHistory(history, tone, kind, search) {
  const query = search.trim().toLowerCase();
  return history.filter(entry => (tone === "all" || entry.tone === tone) &&
    (kind === "all" || entry.kind === kind) &&
    (entry.label + " " + entry.receivedAt).toLowerCase().includes(query));
}
// Allowlist only protocol-defined enums, timestamps, counts and booleans.
// Never copy raw backend messages, account fields, arbitrary metadata or errors.
const knownState = value => typeof value === "string" && Object.hasOwn(STATES, value) ? value : null;
const knownBool = value => typeof value === "boolean" ? value : null;
const knownCount = value => Number.isSafeInteger(value) && value >= 0 ? value : null;
const localDate = value => typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : null;
const localTime = value => typeof value === "string" && /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? value : null;
const eventTime = value => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/.test(value) ? value : null;
export function diagnosticReport(status, telemetry, version, capturedAt) {
  const rows = Array.isArray(telemetry?.rows) ? telemetry.rows : [];
  return {
    schema: "tyut-keepalive-diagnostic-v1",
    capturedAt: eventTime(capturedAt),
    appVersion: typeof version === "string" && /^\d+\.\d+\.\d+$/.test(version) ? version : null,
    status: status ? {
      state: knownState(status.state),
      paused: knownBool(status.paused),
      sleeping: knownBool(status.sleeping),
      lastCheck: localTime(status.lastCheck),
      lastLogin: localTime(status.lastLogin),
    } : null,
    telemetry: telemetry ? {
      date: localDate(telemetry.daily?.date),
      authConfirmed: knownCount(telemetry.daily?.authConfirmed),
      interruptions: knownCount(telemetry.daily?.interruptions),
      observedOnlineSeconds: knownCount(telemetry.observedOnlineSeconds),
      notificationsEnabled: knownBool(telemetry.notificationsEnabled),
      persistenceOk: knownBool(telemetry.persistenceOk),
      events: rows.slice(0, 200).flatMap(row => {
        const time = eventTime(row?.time);
        return time && DIAGNOSTIC_EVENT_KINDS.includes(row?.kind) ? [{time, kind: row.kind}] : [];
      }),
    } : null,
  };
}

/** A display-only countdown: the Rust scheduler remains the sole task owner. */
export function countdownFrame(schedule, kind, elapsedMs) {
  if (!schedule || schedule.kind !== kind ||
      !Number.isFinite(schedule.totalMs) || schedule.totalMs <= 0 ||
      !Number.isFinite(schedule.remainingMs) || !Number.isFinite(elapsedMs)) {
    return { active: false, seconds: 0, fraction: 0, label: "—" };
  }
  const remaining = Math.max(0, Math.min(schedule.totalMs, schedule.remainingMs - Math.max(0, elapsedMs)));
  const seconds = Math.ceil(remaining / 1000);
  const label = String(Math.floor(seconds / 60)).padStart(2, "0") + ":" + String(seconds % 60).padStart(2, "0");
  return { active: true, seconds, fraction: remaining / schedule.totalMs, label };
}
