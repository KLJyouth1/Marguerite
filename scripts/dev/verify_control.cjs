// 检查完成门禁误判是否与插话相关（对照：同任务无插话跑一次）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '开发一个个人博客系统：需要文章发布、评论、标签分类。请给出完整技术方案、数据库表设计和实施步骤清单。',
      sessionKey: 'ws.inject.control',
    }}));
    return;
  }
  if (m.type === 'event') {
    if (m.event === 'session.event.done') { console.log('CONTROL done, len:', (m.payload.text_snapshot||'').length); process.exit(0); }
    if (m.event === 'session.event.error') { console.log('CONTROL ERR:', JSON.stringify(m.payload).slice(0,130)); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
