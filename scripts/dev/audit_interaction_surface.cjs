// 审计 v4：features 在 res 顶层
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c' && m.features) {
    const methods = m.features.methods;
    const interesting = methods.filter(x => /steer|approval|progress|correct|clarify|queue|cancel|stop/.test(x));
    console.log('总方法数:', methods.length);
    console.log('交互相关:', JSON.stringify(interesting));
    process.exit(0);
  }
});
setTimeout(() => { console.log('未捕获'); process.exit(1); }, 8000);
