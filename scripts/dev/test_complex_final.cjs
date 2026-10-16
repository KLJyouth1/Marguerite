// 最终复测：完整复杂任务（自动拆解 + 多步骤执行）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0; let lastEv = '';
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: { message: '帮我规划一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤', sessionKey: 'ws.cx3' } }));
  }
  if (m.type === 'event') {
    const dt = ((Date.now() - t0) / 1000).toFixed(0);
    if (m.event === 'session.event.plan_revision' && lastEv !== 'plan_revision') { process.stdout.write(`[${dt}s] plan_revision（自动拆解）\n`); }
    if (m.event === 'session.event.plan_run' && lastEv !== 'plan_run') { process.stdout.write(`[${dt}s] plan_run\n`); }
    lastEv = m.event;
    if (m.event === 'session.event.done') {
      process.stdout.write(`[${dt}s] DONE 长度=${(m.payload.text_snapshot || '').length}\n`);
      process.stdout.write('内容预览: ' + (m.payload.text_snapshot || '').replace(/\n/g, ' ').slice(0, 200) + '\n');
      process.exit(0);
    }
    if (m.event === 'session.event.error') {
      process.stdout.write(`[${dt}s] ERROR: ${JSON.stringify(m.payload).slice(0, 180)}\n`);
      process.exit(0);
    }
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT-400s'); process.exit(1); }, 400000);
