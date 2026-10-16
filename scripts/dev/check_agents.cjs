// agents.list 返回结构核验（上轮"结构待核"）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') ws.send(JSON.stringify({ type: 'req', id: 'a', method: 'agents.list', params: {} }));
  if (m.type === 'res' && m.id === 'a') {
    const p = m.payload;
    process.stdout.write('type=' + Array.isArray(p) ? '' : '');
    process.stdout.write('\npayload=' + JSON.stringify(p).slice(0, 260) + '\n');
    process.exit(0);
  }
});
setTimeout(() => { process.stdout.write('timeout\n'); process.exit(1); }, 10000);
