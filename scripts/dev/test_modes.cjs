// 功能实测 A：三模式（plan / agent / bloom）切换 + 任务执行
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `m-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 90000);
  });
}
function chat(message, sessionKey) {
  return new Promise((resolve) => {
    const onEvent = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'event' && m.event === 'session.event.done' && m.payload.session_key === sessionKey) {
        ws.off('message', onEvent);
        resolve({ ok: true, text: m.payload.text_snapshot || m.payload.text || '', reason: m.payload.reason });
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent);
        resolve({ ok: false, error: JSON.stringify(m.payload).slice(0, 150) });
      }
    };
    ws.on('message', onEvent);
    ws.send(JSON.stringify({ type: 'req', id: `chat-${++id}`, method: 'chat.send', params: { message, sessionKey } }));
    setTimeout(() => { ws.off('message', onEvent); resolve({ ok: false, error: 'timeout-90s' }); }, 90000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
  else if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
ws.on('error', (e) => console.log('WS ERROR:', e.message));

async function run() {
  const out = {};
  // 当前模式
  out.initial_mode = ((await rpc('mode.get')).payload || {});
  const modes = ['plan', 'agent', 'bloom'];
  for (const mode of modes) {
    const sw = await rpc('mode.switch', { kind: mode });
    const after = (await rpc('mode.get')).payload || {};
    const r = await chat(`用一句话说明什么是${mode === 'plan' ? '计划模式' : mode === 'agent' ? '智能体' : '多模型协同'}`, `ws.mode.${mode}`);
    out[mode] = {
      switch_ok: !!(sw && sw.ok),
      effective_kind: after.kind,
      chat_ok: r.ok,
      reply: (r.text || r.error || '').slice(0, 80),
    };
  }
  // 还原 agent 模式
  await rpc('mode.switch', { kind: 'agent' });
  console.log(JSON.stringify(out, null, 2));
  process.exit(0);
}
