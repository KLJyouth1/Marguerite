// v4：换任务措辞避开完成门禁误判，验证插话注入位置
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let taskId = '';
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '开发一个个人博客系统：需要文章发布、评论、标签分类。请给出完整技术方案、数据库表设计和实施步骤清单。',
      sessionKey: 'ws.inject.v4',
    }}));
    setTimeout(() => {
      if (!taskId) return;
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.v4',
        message: '技术选型必须优先考虑国内可访问的服务商如阿里云',
        expected_turn_id: taskId,
        client_request_id: 'cr-v4-' + Date.now(),
        client_message_id: 'cm-v4-' + Date.now(),
      }}));
    }, 30000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    if (m.event === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      const parts = text.split(/运行期间收到你的补充/);
      console.log('总长:', text.length, '| 分块:', parts.length);
      console.log('附录前正文含「阿里云」:', parts.length > 1 ? /阿里云/.test(parts[0]) : /阿里云/.test(text));
      console.log('全文含「阿里云」:', /阿里云/.test(text));
      process.exit(0);
    }
    if (m.event === 'session.event.error') { console.log('ERR:', JSON.stringify(m.payload).slice(0,130)); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
