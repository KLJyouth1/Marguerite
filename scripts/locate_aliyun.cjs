// 精确定位「阿里云」在回复中的位置（正文 or 附录 or 步骤结论聚合）
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
      sessionKey: 'ws.inject.locate',
    }}));
    setTimeout(() => {
      if (!taskId) return;
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.locate',
        message: '技术选型必须优先考虑国内可访问的服务商如阿里云',
        expected_turn_id: taskId,
        client_request_id: 'cr-lo-' + Date.now(),
        client_message_id: 'cm-lo-' + Date.now(),
      }}));
    }, 30000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    if (m.event === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      // 打印所有含阿里云的行及前后文
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        if (/阿里云/.test(line)) {
          console.log(`--- L${i} ---`);
          console.log(lines.slice(Math.max(0,i-1), i+2).join('\n').slice(0, 300));
        }
      });
      console.log('=== 尾部 200 字 ===');
      console.log(text.slice(-200));
      process.exit(0);
    }
    if (m.event === 'session.event.error') { console.log('ERR'); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
