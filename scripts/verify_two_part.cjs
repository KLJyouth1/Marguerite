// 检查 lead 装配点：单轮路径的插话是否真的进入 build_model_messages
// （单轮任务无工具循环 → dispatch 只调一次 process_input → 只有一次装配机会）
// 验证方式：单轮长回复任务 + 插话，看回复是否体现插话内容
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let taskId = '';
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    // 用一个明确要求「分两部分回答」的任务：第一部分先答，第二部分等插话后答
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '请写一篇关于茶文化的文章，第一部分先介绍绿茶。写完第一部分后暂停，等待我的补充再写第二部分。',
      sessionKey: 'ws.inject.two-part',
    }}));
    setTimeout(() => {
      if (!taskId) return;
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.two-part',
        message: '第二部分请介绍普洱茶，重点讲云南产地',
        expected_turn_id: taskId,
        client_request_id: 'cr-tp-' + Date.now(),
        client_message_id: 'cm-tp-' + Date.now(),
      }}));
    }, 8000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    if (m.event === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      console.log('done len:', text.length);
      console.log('含普洱:', /普洱/.test(text));
      console.log('含云南:', /云南/.test(text));
      console.log('--- 尾部 ---');
      console.log(text.slice(-300));
      process.exit(0);
    }
    if (m.event === 'session.event.error') { console.log('ERR'); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 240000);
