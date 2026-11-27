import { normalizeTheme, readTheme, saveTheme, describeState, keepaliveControls, filterHistory, diagnosticReport, countdownFrame } from "./ui-state.js";
const $ = id => document.getElementById(id);
const pages = ["overview", "logs", "settings", "about"];
const controls = ["reconnect", "saveCredentials", "reset", "autostart", "hide", "quit"];
const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;
let ready = false;
let updatesEnabled = false;
let updateBusy = false;
let pendingUpdate = null;
let currentStatus = null;
let scheduleReceivedAt = 0;
let countdownTimer = null;
function paintCountdowns() {
  const elapsed = performance.now() - scheduleReceivedAt;
  for (const kind of ["check", "retry"]) {
    const frame = countdownFrame(currentStatus?.schedule, kind, elapsed);
    $(kind + "Ring").setAttribute("stroke-dasharray", (frame.fraction * 100) + " 100");
    $(kind + "Time").textContent = frame.label;
    $(kind + "Dial").dataset.active = String(frame.active);
    $(kind + "Hint").textContent = frame.active
      ? (frame.seconds === 0 ? "等待后台执行" : "剩余时间")
      : "待机";
  }
}
function syncCountdownTimer() {
  if (countdownTimer !== null) clearInterval(countdownTimer);
  countdownTimer = null;
  paintCountdowns();
  if (!document.hidden && currentStatus?.schedule) {
    countdownTimer = setInterval(paintCountdowns, 1000);
  }
}

let autostartKnown = false;
let busy = false;
let statusRevision = 0;
let requestRevision = 0;
let history = [];
let telemetryRevision = 0;
let telemetryKnown = false;
function notice(message, error = false) {
  const el = $("notice");
  el.textContent = String(message || "");
  el.dataset.error = String(error);
  el.hidden = !message;
}
function setPage(page) {
  if (!pages.includes(page)) return;
  for (const name of pages) $("page-" + name).hidden = name !== page;
  for (const button of document.querySelectorAll("[data-page]")) {
    if (button.dataset.page === page) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
  if (page === "logs") { renderHistory(); if (ready) refreshTelemetry(); }
}
for (const button of document.querySelectorAll("[data-page]")) {
  button.addEventListener("click", () => setPage(button.dataset.page));
}
$("setup").addEventListener("click", () => {
  setPage("settings");
  $("accountInput").focus();
});
function applyTheme(theme) {
  const value = normalizeTheme(theme);
  document.documentElement.dataset.theme = value;
  $("theme").value = value;
}
let storage;
try { storage = window.localStorage; } catch { storage = null; }
applyTheme(readTheme(storage));
$("theme").addEventListener("change", () => {
  applyTheme($("theme").value);
  if (!saveTheme(storage, $("theme").value)) notice("主题已切换，但无法保存偏好设置", true);
});
function updateControls() {
  for (const id of controls) $(id).disabled = !ready || busy || updateBusy || (id === "autostart" && !autostartKnown);
  $("notifications").disabled = !ready || busy || updateBusy || !telemetryKnown;
  $("generateDiagnostic").disabled = !ready || busy || updateBusy;
  $("copyDiagnostic").disabled = busy || updateBusy || !$("diagnosticText").value;
  $("checkUpdate").disabled = !ready || busy || updateBusy || !updatesEnabled;
  $("installUpdate").disabled = !ready || busy || updateBusy || !updatesEnabled || !pendingUpdate;
  const policy = keepaliveControls(currentStatus);
  $("pauseToggle").disabled = !ready || busy || updateBusy || !currentStatus;
  $("pauseToggle").textContent = policy.paused ? "恢复保活" : "暂停保活";
  $("reconnect").disabled = !ready || busy || updateBusy || !policy.reconnect;
}
function render(status) {
  if (!status || typeof status !== "object") return;
  currentStatus = status;
  scheduleReceivedAt = performance.now();
  syncCountdownTimer();
  const [label, tone] = describeState(status.state);
  $("connectionCard").dataset.tone = tone;
  $("state").textContent = label;
  $("account").textContent = status.account || "未配置";
  $("lastCheck").textContent = status.lastCheck || "—";
  $("lastLogin").textContent = status.lastLogin || "—";
  $("message").textContent = status.message || "—";
  updateControls();
}
function renderHistory() {
  const entries = filterHistory(history, $("logFilter").value, $("logKind").value, $("logSearch").value);
  $("logCount").textContent = telemetryKnown ? "显示 " + entries.length + " / " + history.length + " 条记录" : "记录未读取或已过期";
  $("historyEmpty").textContent = !telemetryKnown ? "运行记录暂不可用" : history.length ? "没有匹配的事件，请重置筛选" : "暂无状态事件";
  const fragment = document.createDocumentFragment();
  for (const entry of entries) {
    const li = document.createElement("li");
    const time = document.createElement("time");
    time.dateTime = entry.receivedAt;
    time.textContent = new Date(entry.receivedAt).toLocaleString();
    const label = document.createElement("strong");
    label.textContent = entry.label;
    label.dataset.tone = entry.tone;
    const message = document.createElement("span");
    message.textContent = entry.message;
    li.append(time, label, message);
    fragment.append(li);
  }
  $("historyList").replaceChildren(fragment);
  $("historyEmpty").hidden = entries.length > 0;
}
$("logFilter").addEventListener("change", renderHistory);
$("logKind").addEventListener("change", renderHistory);
$("resetLogFilters").addEventListener("click", () => {
  $("logFilter").value = "all";
  $("logKind").value = "all";
  $("logSearch").value = "";
  renderHistory();
});
$("logSearch").addEventListener("input", renderHistory);
$("refreshHistory").addEventListener("click", refreshTelemetry);
const specialEvents = {
  "auth-confirmed": ["认证接口确认成功或已在线", "success"],
  "connection-lost": ["检测到连接中断", "warning"],
  "connection-restored": ["检测到连接恢复", "success"],
  "credentials-changed": ["凭据配置已变更或正在清除", "neutral"],
  "system-suspended": ["系统进入休眠", "neutral"],
  "system-resumed": ["系统已唤醒", "neutral"],
  "network-changed": ["网络接口变化，重新检查", "active"],
};
function duration(seconds) {
  if (seconds == null) return "—";
  const n = Math.max(0, Math.floor(seconds));
  return Math.floor(n / 3600) + ":" + String(Math.floor(n / 60) % 60).padStart(2, "0") + ":" + String(n % 60).padStart(2, "0");
}
async function refreshTelemetry() {
  if (!ready) return;
  const revision = ++telemetryRevision;
  try {
    const data = await invoke("get_telemetry");
    if (revision !== telemetryRevision) return;
    $("authCount").textContent = String(data.daily.authConfirmed);
    $("outageCount").textContent = String(data.daily.interruptions);
    $("onlineDuration").textContent = duration(data.observedOnlineSeconds);
    $("statsDate").textContent = data.daily.date;
    $("storageStatus").textContent = data.persistenceOk ? "日志已启用本地持久化" : "日志存储异常，统计可能不完整";
    $("notifications").checked = data.notificationsEnabled;
    telemetryKnown = true;
    history = data.rows.map(row => {
      const [label, tone] = Object.hasOwn(specialEvents, row.kind) ? specialEvents[row.kind] : describeState(row.kind);
      return {kind: row.kind, label, tone, message: "", receivedAt: row.time};
    });
    if (!$("page-logs").hidden) renderHistory();
  } catch (error) {
    if (revision !== telemetryRevision) return;
    telemetryKnown = false;
    $("storageStatus").textContent = "无法读取运行记录";
    if (!$("page-logs").hidden) renderHistory();
  } finally { updateControls(); }
}
$("notifications").addEventListener("change", () => action(async () => {
  const enabled = $("notifications").checked;
  ++telemetryRevision;
  try {
    const actual = await invoke("set_notifications", { enabled });
    $("notifications").checked = actual;
    notice(actual ? "系统通知已启用" : "系统通知已关闭");
  } catch (error) {
    await refreshTelemetry();
    throw error;
  }
}));
async function refreshStatus() {
  const request = ++requestRevision;
  const eventRevision = statusRevision;
  const snapshot = await invoke("get_status");
  if (request === requestRevision && eventRevision === statusRevision) render(snapshot);
}
let autostartRevision = 0;
async function refreshAutostart() {
  const revision = ++autostartRevision;
  try {
    const enabled = await invoke("is_autostart");
    if (revision !== autostartRevision) return;
    $("autostart").checked = Boolean(enabled);
    $("autostartText").textContent = enabled ? "系统设置：已开启" : "系统设置：已关闭";
    autostartKnown = true;
  } catch {
    if (revision !== autostartRevision) return;
    $("autostartText").textContent = "无法读取自启设置";
    autostartKnown = false;
  } finally { updateControls(); }
}
async function refresh() {
  if (!ready || busy) return;
  try { await Promise.all([refreshStatus(), refreshAutostart(), refreshTelemetry()]); }
  catch (error) { notice("读取状态失败：" + String(error), true); }
}
async function action(fn) {
  if (!ready || busy || updateBusy) return;
  busy = true;
  updateControls();
  try { await fn(); } catch (error) { notice(String(error), true); }
  finally { busy = false; updateControls(); }
}
$("generateDiagnostic").addEventListener("click", () => action(async () => {
  $("diagnosticText").value = "";
  $("diagnosticStatus").textContent = "正在读取本地快照";
  const [status, telemetry, version] = await Promise.allSettled([
    invoke("get_status"), invoke("get_telemetry"),
    Promise.resolve().then(() => tauri.app?.getVersion?.()),
  ]);
  const value = result => result.status === "fulfilled" ? result.value : null;
  if (!value(status) && !value(telemetry)) {
    $("diagnosticStatus").textContent = "后台状态和运行记录均不可用，未生成报告";
    return;
  }
  const report = diagnosticReport(value(status), value(telemetry), value(version), new Date().toISOString());
  $("diagnosticText").value = JSON.stringify(report, null, 2);
  $("diagnosticStatus").textContent = value(status) && value(telemetry)
    ? "报告已生成，请预览后复制"
    : "报告已生成，但部分后台数据不可用";
}));
$("copyDiagnostic").addEventListener("click", async () => {
  const output = $("diagnosticText");
  if (!output.value || busy || updateBusy) return;
  try {
    // No native clipboard plugin or new IPC permission. Some WebViews
    // deny clipboard access; selecting the text still permits manual copy.
    if (!navigator.clipboard?.writeText) throw Error("clipboard unavailable");
    await navigator.clipboard.writeText(output.value);
    $("diagnosticStatus").textContent = "报告已复制，请检查后分享";
  } catch {
    output.focus();
    output.select();
    $("diagnosticStatus").textContent = "系统未允许自动复制，已选中报告，请按 Ctrl+C（macOS：Command+C）";
  }
});
$("pauseToggle").addEventListener("click", () => action(async () => {
  const paused = !keepaliveControls(currentStatus).paused;
  await invoke("set_paused", { paused });
  await refreshStatus();
  notice(paused ? "已暂停自动保活" : "已恢复保活设置");
}));
$("reconnect").addEventListener("click", () => action(async () => {
  await invoke("reconnect");
  notice("已发送重连请求");
}));
$("credentials").addEventListener("submit", event => {
  event.preventDefault();
  action(async () => {
    const account = $("accountInput").value.trim();
    const password = $("passwordInput").value;
    if (!account || !password) { notice("请输入账号和密码", true); return; }
    try { await invoke("setup_credentials", { account, password }); }
    finally { $("passwordInput").value = ""; }
    $("accountInput").value = "";
    setPage("overview");
    await refreshStatus();
    notice(keepaliveControls(currentStatus).paused ? "凭据已保存，保活仍处于暂停状态" : "凭据已保存，后台将按状态处理");
  });
});
$("reset").addEventListener("click", () => action(async () => {
  if (!window.confirm("清除本机保存的校园网凭据？此操作不会主动注销当前网络。")) return;
  await invoke("reset_credentials");
  $("credentials").reset();
  notice("凭据已删除，请重新设置");
}));
$("autostart").addEventListener("change", () => action(async () => {
  const enabled = $("autostart").checked;
  ++autostartRevision;
  try {
    const actual = await invoke("set_autostart", { enabled });
    $("autostart").checked = Boolean(actual);
    $("autostartText").textContent = actual ? "系统设置：已开启" : "系统设置：已关闭";
    notice("开机自启设置已更新");
  } catch (error) {
    await refreshAutostart();
    throw error;
  }
}));
$("hide").addEventListener("click", () => action(async () => {
  await invoke("hide_window");
  notice("已隐藏窗口，后台服务继续运行");
}));
$("quit").addEventListener("click", () => action(async () => {
  if (window.confirm("彻底退出并停止后台保活？")) await invoke("quit");
}));
async function checkUpdate() {
  if (!ready || busy || updateBusy || !updatesEnabled) return;
  updateBusy = true;
  pendingUpdate = null;
  updateControls();
  $("updateStatus").textContent = "正在检查更新";
  $("updateNotes").hidden = true;
  $("updateNotes").textContent = "";
  try {
    pendingUpdate = await invoke("check_update");
    $("updateStatus").textContent = pendingUpdate ? "发现新版 v" + pendingUpdate.version : "当前没有可用的更新";
    $("updateNotes").textContent = pendingUpdate?.notes || "";
    $("updateNotes").hidden = !$("updateNotes").textContent;
  } catch (error) {
    $("updateStatus").textContent = String(error);
  } finally { updateBusy = false; updateControls(); }
}
$("checkUpdate").addEventListener("click", checkUpdate);
$("installUpdate").addEventListener("click", async () => {
  if (!ready || busy || updateBusy || !pendingUpdate) return;
  const version = pendingUpdate.version;
  if (!window.confirm("下载并安装签名更新 v" + version + "？安装成功后应用可能重新启动。")) return;
  updateBusy = true;
  pendingUpdate = null;
  updateControls();
  $("updateStatus").textContent = "正在下载、验证签名并安装";
  try {
    await invoke("install_update", { version });
    $("updateStatus").textContent = "安装操作已完成，请按系统提示重新启动";
  } catch (error) {
    $("updateStatus").textContent = String(error);
  } finally { updateBusy = false; updateControls(); }
});
async function initUpdates() {
  try {
    updatesEnabled = await invoke("update_enabled");
    $("updateStatus").textContent = updatesEnabled ? "签名更新可用" : "签名更新未启用：尚未配置可信公钥";
    updateControls();
    if (updatesEnabled) await checkUpdate();
  } catch {
    $("updateStatus").textContent = "无法读取签名更新配置";
  }
}
document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    if (countdownTimer !== null) clearInterval(countdownTimer);
    countdownTimer = null;
  } else {
    // A new backend snapshot is authoritative after sleep or a hidden window.
    if (currentStatus) currentStatus = { ...currentStatus, schedule: null };
    syncCountdownTimer();
    refresh();
  }
});
window.addEventListener("focus", refresh);
async function init() {
  if (!invoke || !listen) {
    $("serviceText").textContent = "后台未连接";
    notice("当前未连接 Tauri 后台，界面可预览，网络操作不可用。", true);
    return;
  }
  try {
    await listen("status", event => {
      statusRevision += 1;
      render(event.payload);
      // Journal events are delivered separately, including while the window is hidden.
    });
    await listen("telemetry-changed", () => { if (ready && !document.hidden) refreshTelemetry(); });
    await listen("open-settings", () => setPage("settings"));
    await refreshStatus();
    ready = true;
    $("serviceText").textContent = "后台服务已连接";
    updateControls();
    await Promise.all([refreshAutostart(), refreshTelemetry()]);
    void initUpdates();
    try {
      const version = await tauri.app?.getVersion?.();
      if (version) $("appVersion").textContent = "v" + version;
    } catch { /* keep the truthful generic development label */ }
  } catch (error) {
    ready = false;
    updateControls();
    $("serviceText").textContent = "后台连接失败";
    notice("初始化失败：" + String(error), true);
  }
}
init();
