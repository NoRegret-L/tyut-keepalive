import test from "node:test";
import assert from "node:assert/strict";
import { normalizeTheme, readTheme, saveTheme, describeState, addHistory, HISTORY_LIMIT, keepaliveControls, filterHistory, diagnosticReport, countdownFrame } from "../dist/ui-state.js";

test("theme supports three modes and handles corrupt preferences", () => {
  for (const theme of ["system", "light", "dark"]) assert.equal(normalizeTheme(theme), theme);
  for (const value of [null, undefined, "unknown", "__proto__"]) assert.equal(normalizeTheme(value), "system");
  const values = new Map();
  const storage = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) };
  assert.equal(readTheme(storage), "system");
  assert.equal(saveTheme(storage, "dark"), true);
  assert.equal(readTheme(storage), "dark");
  assert.equal(readTheme(null), "system");
  assert.equal(saveTheme(null, "light"), false);
});
test("every backend connection state has a localized presentation", () => {
  for (const state of ["initializing", "need-setup", "credential-error", "checking", "logging-in", "online", "offline", "invalid-credentials"]) {
    assert.notEqual(describeState(state)[0], "未知状态");
  }
  assert.equal(describeState("online")[1], "success");
  assert.equal(describeState("invalid-credentials")[1], "danger");
  assert.equal(describeState("__proto__")[0], "未知状态");
});
test("UI history records actual transitions, deduplicates and bounds memory", () => {
  const timestamp = "2026-11-26T00:00:00Z";
  const event = { state: "online", message: "网络正常", account: "never copy account" };
  const first = addHistory([], event, timestamp);
  assert.equal(first.length, 1);
  assert.equal(Object.hasOwn(first[0], "account"), false);
  assert.equal(addHistory(first, event, timestamp), first);
  let history = first;
  for (let i = 0; i <= HISTORY_LIMIT; i++) history = addHistory(history, { state: "offline", message: String(i) }, timestamp);
  assert.equal(history.length, HISTORY_LIMIT);
  assert.equal(history[0].message, String(HISTORY_LIMIT));
});

test("pause and sleep block reconnect, explicit password retry remains available", () => {
  assert.equal(keepaliveControls(null).reconnect, false);
  assert.equal(keepaliveControls({state:"online",paused:true}).reconnect, false);
  assert.equal(keepaliveControls({state:"online",sleeping:true}).reconnect, false);
  assert.equal(keepaliveControls({state:"invalid-credentials"}).reconnect, true);
  assert.equal(keepaliveControls({state:"credential-error"}).reconnect, false);
  assert.equal(keepaliveControls({state:"online"}).reconnect, true);
  assert.equal(keepaliveControls({state:"paused",paused:true}).paused, true);
  assert.notEqual(describeState("suspended")[0], "未知状态");
});

test("log filters intersect and reset returns all rows", () => {
  const entries = [
    {kind:"connection-lost",tone:"warning",label:"连接中断",receivedAt:"2026-11-27"},
    {kind:"connection-restored",tone:"success",label:"连接恢复",receivedAt:"2026-11-28"},
  ];
  assert.equal(filterHistory(entries,"all","all","").length,2);
  assert.equal(filterHistory(entries,"all","connection-lost","2026-11-27").length,1);
  assert.equal(filterHistory(entries,"success","connection-lost","").length,0);
  assert.equal(filterHistory(entries,"all","all","不存在").length,0);
});
test("diagnostic allowlist excludes credentials, addresses, raw messages and unknown event data", () => {
  const secret = "DO_NOT_LEAK_192.168.1.1";
  const time = "2026-11-27T14:24:00+08:00";
  const status = {state:"online",paused:false,sleeping:false,account:secret,password:secret,message:secret,lastCheck:"2026-11-27 14:24:00",lastLogin:secret};
  const telemetry = {daily:{date:"2026-11-27",authConfirmed:2,interruptions:1},observedOnlineSeconds:30,notificationsEnabled:false,persistenceOk:true,rows:[{time,kind:"connection-restored",message:secret},{time:secret,kind:"online"},{time,kind:secret}],path:secret};
  const report = diagnosticReport(status,telemetry,"0.4.0",time);
  assert.equal(JSON.stringify(report).includes(secret),false);
  assert.equal(report.status.lastLogin,null);
  assert.equal(report.status.state,"online");
  assert.equal(report.telemetry.events.length,1);
  assert.equal(report.telemetry.events[0].kind,"connection-restored");
  assert.equal(report.telemetry.authConfirmed,2);
  assert.equal(report.appVersion,"0.4.0");
});
test("diagnostic handles unavailable and malformed data and bounds report length", () => {
  const time = "2026-11-27T14:24:00Z";
  const empty = diagnosticReport(null,null,"not a version",time);
  assert.equal(empty.status,null);
  assert.equal(empty.telemetry,null);
  assert.equal(empty.appVersion,null);
  const report = diagnosticReport({state:"__proto__",paused:"true"}, {daily:{authConfirmed:-1},rows:Array.from({length:220},()=>({kind:"online",time}))},"0.4.0",time);
  assert.equal(report.status.state,null);
  assert.equal(report.status.paused,null);
  assert.equal(report.telemetry.authConfirmed,null);
  assert.equal(report.telemetry.events.length,200);
});

test("countdown derives progress from scheduler snapshot, never from retry guesses", () => {
  const schedule = { kind: "check", totalMs: 600000, remainingMs: 42000 };
  assert.equal(countdownFrame(schedule, "check", 1000).label, "00:41");
  assert.equal(countdownFrame(schedule, "check", 1000).fraction, 41000 / 600000);
  assert.equal(countdownFrame(schedule, "retry", 0).active, false);
  assert.equal(countdownFrame(schedule, "check", 100000).seconds, 0);
  assert.equal(countdownFrame(null, "check", 0).label, "—");
  assert.equal(countdownFrame({kind:"retry",totalMs:10000,remainingMs:10000},"retry",0).fraction,1);
  assert.equal(countdownFrame({kind:"retry",totalMs:0,remainingMs:0},"retry",0).active,false);
});
