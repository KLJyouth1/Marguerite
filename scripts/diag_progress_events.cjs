// 诊断 step_progress：打印全部事件类型
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0;
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: { message: '帮我开发一个完整的个人博客系统：包含文章发布、评论、标签分类三大功能的技术方案与实施步骤', sessionKey: 'ws.prog.v6' } }));
  }
  if (m.type === 'event') {
    const dt = ((Date.now() - t0) / 1000).toFixed(0);
    const ev = (m.event || '').replace('session.event.', '');
    if (!['thinking', 'tick', 'usage', 'text_delta'].includes(ev)) {
      console.log(`[${dt}s] ${ev}`, ev === 'progress' ? JSON.stringify(m.payload).slice(0, 160) : '');
      if (m.event === 'session.event.done' || m.event === 'session.event.error') process.exit(0);
    }
  }
});
setTimeout(() => process.exit(1), 480000);
