// v3.3 插话端到端验证：运行中 steer → 入队 → turn 结束注入
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0; let taskId = ''; let steered = false;
const out = { steer_accepted_seen: false, steers_applied_seen: false, injected_in_reply: false, done_len: 0 };
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    // 复杂任务 → 走计划路径（长执行窗口，留足插话时间）
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '帮我规划一个完整的个人博客系统：文章发布、评论、标签分类的技术方案与实施步骤',
      sessionKey: 'ws.steer.e2e',
    }}));
    // 15 秒后发插话（turn 已在运行中）
    setTimeout(() => {
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.steer.e2e',
        message: '补充要求：技术选型优先考虑国内可访问的服务商',
        expected_turn_id: taskId,
        client_request_id: 'cr-qa-' + Date.now(),
        client_message_id: 'cm-qa-' + Date.now(),
        surface_id: 'webui',
      }}));
    }, 15000);
    return;
  }
  if (m.type === 'res' && m.id === 't') {
    taskId = (m.payload && m.payload.taskId) || '';
    process.stdout.write('[send] taskId=' + taskId.slice(0, 30) + '\n');
  }
  if (m.type === 'res' && m.id === 'st') {
    process.stdout.write('[steer] accepted=' + (m.payload && m.payload.accepted) + ' disposition=' + (m.payload && m.payload.disposition) + '\n');
    steered = !!(m.payload && m.payload.accepted);
  }
  if (m.type === 'event') {
    const ev = m.event || '';
    if (ev.includes('status') && m.payload && m.payload.kind === 'steer_accepted') out.steer_accepted_seen = true;
    if (ev.includes('status') && m.payload && m.payload.kind === 'steers_applied') out.steers_applied_seen = true;
    if (ev === 'session.event.done') {
      const text = (m.payload.text_snapshot || '');
      out.done_len = text.length;
      out.injected_in_reply = /用户插话/.test(text) && /国内可访问/.test(text);
      console.log(JSON.stringify(out, null, 2));
      console.log('STEER-E2E:', (out.injected_in_reply && steered) ? 'PASS' : 'FAIL');
      process.exit(0);
    }
    if (ev === 'session.event.error') {
      console.log('ERROR:', JSON.stringify(m.payload).slice(0, 160));
      process.exit(1);
    }
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); console.log(JSON.stringify(out)); process.exit(1); }, 420000);
