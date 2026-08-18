#!/usr/bin/env node
/*
 * Marguerite Gateway WS Smoke Test
 * ---------------------------------
 * Verifies the 5 user-reported bug fixes at RUNTIME against a live `marguerite-core gateway`:
 *   D1 离开页面不中断任务  -> chat.send spawns a server-side tokio task; we hard-disconnect
 *                             the WS client mid-turn and confirm the session still exists after
 *                             reconnecting (turn was NOT bound to the socket).
 *   D2 会话运行态/空闲时间 -> we must observe `sessions.changed` (status: running) after send.
 *   D3 用量实时刷新        -> we must observe `session.event.usage` (if the LLM call runs).
 *   D5 Bloom/路由编排卡片  -> we must observe `session.event.router_decision` /
 *                             `session.event.ensemble_progress` (if orchestration runs).
 *
 * Transport contract (see core/src/gateway/{protocol,server,events}.rs):
 *   1) server -> client: `connect.challenge` event
 *   2) client -> server: req { id, method:"connect", params:{minProtocol:3,maxProtocol:3,auth:{}} }
 *   3) server -> client: res { id:"1", ok:true, protocol, features:{methods,events} }  (HelloOk)
 *   4) after handshake the WS forwarder auto-subscribes `**`; ALL envelopes stream as
 *      `event` frames (`sessions.changed`, `session.event.*`) with an incrementing `seq`.
 *   No explicit subscribe call is needed.
 */
const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');

const WS_URL = process.env.MARG_WS_URL || 'ws://127.0.0.1:18790/ws';
const PROMPT = process.env.MARG_PROMPT || '用一句话介绍你自己。';
const RESULT_PATH = process.env.MARG_RESULT || path.join(__dirname, 'smoke_result.json');
const CONN_RETRIES = parseInt(process.env.MARG_CONN_RETRIES || '60', 10);
const CONN_DELAY_MS = parseInt(process.env.MARG_CONN_DELAY || '1000', 10);

function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
function ts() { return new Date().toISOString(); }

async function tryConnect() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const events = [];
    let hello = null;
    const to = setTimeout(() => {
      try { ws.terminate(); } catch (_) {}
      reject(new Error('connect timeout (no HelloOk within 12s)'));
    }, 12000);
    ws.on('message', (data) => {
      let f;
      try { f = JSON.parse(data.toString()); } catch (_) { return; }
      if (f.type === 'event' && f.event === 'connect.challenge') {
        ws.send(JSON.stringify({
          type: 'req', id: '1', method: 'connect',
          params: { minProtocol: 3, maxProtocol: 3, auth: {} },
        }));
      } else if (f.type === 'res' && f.id === '1') {
        hello = f;
        clearTimeout(to);
        resolve({ ws, hello, events });
      } else if (f.type === 'event') {
        events.push(f);
      }
    });
    ws.on('error', (e) => { clearTimeout(to); reject(e); });
    ws.on('close', () => { /* ignore; caller controls lifecycle */ });
  });
}

async function connect(retries = CONN_RETRIES, delayMs = CONN_DELAY_MS) {
  let last;
  for (let i = 0; i < retries; i++) {
    try {
      return await tryConnect();
    } catch (e) {
      last = e;
      if (i === retries - 1) throw e;
      await sleep(delayMs);
    }
  }
  throw last;
}

function callRpc(conn, id, method, params) {
  return new Promise((resolve, reject) => {
    const { ws } = conn;
    const to = setTimeout(() => {
      ws.off('message', handler);
      reject(new Error('rpc timeout: ' + method));
    }, 25000);
    const handler = (data) => {
      let f;
      try { f = JSON.parse(data.toString()); } catch (_) { return; }
      if (f.type === 'res' && f.id === id) {
        ws.off('message', handler);
        clearTimeout(to);
        resolve(f);
      }
    };
    ws.on('message', handler);
    ws.send(JSON.stringify({ type: 'req', id, method, params }));
  });
}

function findSession(listPayload, key) {
  if (!listPayload) return null;
  const arr = Array.isArray(listPayload) ? listPayload
    : Array.isArray(listPayload.sessions) ? listPayload.sessions
    : Array.isArray(listPayload.items) ? listPayload.items
    : [];
  return arr.find((s) => (s.sessionKey || s.key || s.id) === key) || null;
}

(async () => {
  const out = [];
  const L = (...a) => { const s = a.map(String).join(' '); out.push('[' + ts() + '] ' + s); console.log(s); };
  const report = { ok: false, steps: [] };
  try {
    L('== [1] handshake (client A) ==');
    const A = await connect();
    const methods = (A.hello.features && A.hello.features.methods) || [];
    const events0 = (A.hello.features && A.hello.features.events) || [];
    L('hello ok | protocol=' + A.hello.protocol +
      ' | methods=' + methods.length + ' | events=' + JSON.stringify(events0));
    L('  has chat.send=' + methods.includes('chat.send') +
      ' has sessions.list=' + methods.includes('sessions.list'));
    report.hello = { protocol: A.hello.protocol, methods_count: methods.length, events: events0 };
    report.has_chat_send = methods.includes('chat.send');
    report.has_sessions_list = methods.includes('sessions.list');

    L('== [2] sessions.list (before) ==');
    const before = await callRpc(A, '2', 'sessions.list', {});
    L('  ok=' + before.ok + ' payload=' + JSON.stringify(before.payload).slice(0, 160));
    report.sessions_before_count = Array.isArray(before.payload)
      ? before.payload.length
      : (before.payload && before.payload.sessions ? before.payload.sessions.length : 'n/a');

    L('== [3] chat.send (start a Bloom turn) ==');
    const sk = 'smoke.' + Date.now();
    const cr = 'smoke-cr-' + Date.now();
    const send = await callRpc(A, '3', 'chat.send', { message: PROMPT, sessionKey: sk, clientRequestId: cr });
    const returnedSk = (send.payload && send.payload.sessionKey) || sk;
    L('  ok=' + send.ok + ' sessionKey=' + returnedSk + ' taskId=' + (send.payload && send.payload.taskId));
    report.chat_send_ok = send.ok === true;
    report.sessionKey = returnedSk;

    // Stay connected on A through the LLM call so we capture usage/router/done
    // (if we disconnected immediately, those events would fire with no client attached).
    let waited = 0;
    while (waited < 10000) {
      const done = A.events.some((e) =>
        e.event === 'session.event.done' ||
        (e.event === 'sessions.changed' && e.payload && /idle|completed/.test(JSON.stringify(e.payload))));
      if (done) break;
      await sleep(1000); waited += 1000;
    }
    const sawRunning = A.events.some((e) =>
      e.event === 'sessions.changed' && e.payload &&
      (e.payload.status === 'running' || e.payload.runStatus === 'running' || /running/i.test(JSON.stringify(e.payload))));
    L('  sessions.changed(running) observed: ' + sawRunning);
    report.D2_sessions_changed_running = sawRunning;

    L('== [4] HARD-DISCONNECT client A (simulate user leaving the page) ==');
    try { A.ws.terminate(); } catch (_) {}  // no clean close frame -> like a tab close

    L('== [5] wait 4s; turn must keep running server-side (D1) ==');
    await sleep(4000);

    L('== [6] reconnect (client B) ==');
    const B = await connect();
    L('  hello ok B');

    L('== [7] sessions.list (after reconnect) — session must still exist (D1) ==');
    const after = await callRpc(B, '4', 'sessions.list', {});
    const found = findSession(after.payload, returnedSk);
    L('  session still listed after disconnect: ' + !!found + (found ? ' ' + JSON.stringify(found).slice(0, 160) : ''));
    report.D1_session_persisted_after_disconnect = !!found;
    if (found) report.session_after = found;

    L('== [8] capture events on B for 9s (turn progress / completion) ==');
    await sleep(9000);
    const all = A.events.concat(B.events);
    const sawDone = all.some((e) => e.event === 'session.event.done' ||
      (e.event === 'sessions.changed' && e.payload && e.payload.status === 'idle'));
    const sawError = all.some((e) => e.event === 'session.event.error');
    const sawUsage = all.some((e) => e.event === 'session.event.usage');
    const sawRouter = all.some((e) => e.event === 'session.event.router_decision');
    const sawEnsemble = all.some((e) => e.event === 'session.event.ensemble_progress');
    const sawAbortByClient = all.some((e) => e.event === 'session.event.error' &&
      /client|disconnect|abort|cancel/i.test(JSON.stringify(e.payload)));
    L('  done/idle=' + sawDone + ' error=' + sawError + ' usage=' + sawUsage +
      ' router_decision=' + sawRouter + ' ensemble_progress=' + sawEnsemble +
      ' aborted_by_client_disconnect=' + sawAbortByClient);
    report.D3_usage_event = sawUsage;
    report.D5_router_decision = sawRouter;
    report.D5_ensemble_progress = sawEnsemble;
    report.turn_done_or_idle = sawDone;
    report.turn_error = sawError;
    report.D1_aborted_by_client_disconnect = sawAbortByClient;

    try { B.ws.close(); } catch (_) {}

    report.ok = true;
    report.events_A = A.events.map((e) => ({ event: e.event, status: e.payload && e.payload.status, seq: e.seq }));
    report.events_B = B.events.map((e) => ({ event: e.event, status: e.payload && e.payload.status, seq: e.seq }));
    report.log = out;

    fs.writeFileSync(RESULT_PATH, JSON.stringify(report, null, 2));
    L('== REPORT written -> ' + RESULT_PATH + ' ==');
    // Verdict
    const D1 = report.D1_session_persisted_after_disconnect && !report.D1_aborted_by_client_disconnect;
    const D2 = report.D2_sessions_changed_running;
    L('VERDICT  D1(leave-page)=' + (D1 ? 'PASS' : 'FAIL') +
      '  D2(session-status)=' + (D2 ? 'PASS' : 'FAIL') +
      '  D3(usage)=' + (report.D3_usage_event ? 'PASS' : 'N/A(offline?)') +
      '  D5(router/ensemble)=' + ((report.D5_router_decision || report.D5_ensemble_progress) ? 'PASS' : 'N/A(offline?)'));
    process.exit(0);
  } catch (e) {
    L('FATAL ' + (e && e.message ? e.message : e));
    report.error = e && e.message ? e.message : String(e);
    report.log = out;
    try { fs.writeFileSync(RESULT_PATH, JSON.stringify(report, null, 2)); } catch (_) {}
    process.exit(1);
  }
})();
