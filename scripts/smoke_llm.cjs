// 冒烟：LLM 链路是否真实可用（决定后续功能测试的深度）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('error', (e) => { console.log('WS ERROR:', e.message); process.exit(2); });
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let saw = { thinking: 0, error: [], done: null };
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    ws.send(JSON.stringify({ type: 'req', id: 't1', method: 'chat.send', params: { message: '请只回复两个字：就绪', sessionKey: 'ws.smoke.keycheck' } }));
  }
  if (m.type === 'event') {
    if (m.event === 'session.event.thinking') saw.thinking++;
    if (m.event === 'session.event.error') saw.error.push(JSON.stringify(m.payload).slice(0, 220));
    if (m.event === 'session.event.done') { saw.done = JSON.stringify(m.payload).slice(0, 200); finish(); }
  }
});
function finish() {
  console.log(JSON.stringify(saw, null, 2));
  process.exit(0);
}
setTimeout(() => { console.log('TIMEOUT-15s'); console.log(JSON.stringify(saw, null, 2)); process.exit(1); }, 15000);
