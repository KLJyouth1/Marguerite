// 检查「成都」出现在正文还是附录（区分注入生效模式）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    // 长任务：多轮工具循环，插话在循环边界并入
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '帮我开发一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤',
      sessionKey: 'ws.inject.v3',
    }}));
    setTimeout(() => {
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.v3',
        message: '技术选型必须优先考虑国内可访问的服务商如阿里云',
        expected_turn_id: (ws._taskId) || '',
        client_request_id: 'cr-v3-' + Date.now(),
        client_message_id: 'cm-v3-' + Date.now(),
      }}));
    }, 28000);
    return;
  }
  if (m.type === 'res' && m.id === 't') ws._taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    if (m.event === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      const parts = text.split(/运行期间收到你的补充/);
      console.log('总长:', text.length, '| 区块数:', parts.length);
      if (parts.length > 1) {
        console.log('附录前正文含阿里云:', /阿里云/.test(parts[0]));
        console.log('附录区块:', parts[1].slice(0, 100));
      } else {
        console.log('全文含阿里云:', /阿里云/.test(text));
      }
      process.exit(0);
    }
    if (m.event === 'session.event.error') { console.log('ERR:', JSON.stringify(m.payload).slice(0,130)); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
