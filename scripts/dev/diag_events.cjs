// 定位 120s 超时：拆解步骤的 LLM 调用到底发生了什么（监听全部事件）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0;
const events = [];
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: { message: '帮我规划一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤', sessionKey: 'ws.diag3' } }));
  }
  if (m.type === 'event') {
    const dt = ((Date.now() - t0) / 1000).toFixed(1);
    const ev = m.event || '';
    if (ev !== 'tick' && !events.includes(ev + (m.payload && m.payload.step_id ? ':' + m.payload.step_id : ''))) {
      events.push(ev);
      process.stdout.write(`[${dt}s] ${ev}\n`);
    }
    if (ev === 'session.event.done' || ev === 'session.event.error') {
      process.stdout.write('FINAL: ' + JSON.stringify(m.payload).slice(0, 200) + '\n');
      process.exit(0);
    }
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT-300s'); process.exit(1); }, 300000);
