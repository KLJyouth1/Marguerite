// 诊断：插话后 lead 的行为——监听 step-2 是否有 thinking/输出
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let taskId = ''; let t0 = 0; let steerSent = false;
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '开发一个个人博客系统：需要文章发布、评论、标签分类。请给出完整技术方案、数据库表设计和实施步骤清单。',
      sessionKey: 'ws.inject.diag',
    }}));
    setTimeout(() => {
      if (!taskId) return;
      steerSent = true;
      console.log(`[${((Date.now()-t0)/1000).toFixed(0)}s] >>> 发送插话`);
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.diag',
        message: '技术选型必须优先考虑国内可访问的服务商如阿里云',
        expected_turn_id: taskId,
        client_request_id: 'cr-dg-' + Date.now(),
        client_message_id: 'cm-dg-' + Date.now(),
      }}));
    }, 30000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    const dt = ((Date.now()-t0)/1000).toFixed(0);
    const ev = (m.event||'').replace('session.event.','');
    if (!['thinking','tick','usage','text_delta','tool_call','tool_result'].includes(ev)) {
      console.log(`[${dt}s] ${ev}`, ev==='error' ? JSON.stringify(m.payload).slice(0,110) : '');
    }
    // 统计插话后 thinking 帧数量
    if (steerSent && ev === 'thinking') { global.thinkAfter = (global.thinkAfter||0)+1; }
    if (ev === 'done' || ev === 'error') {
      console.log('插话后 thinking 帧数:', global.thinkAfter || 0);
      process.exit(0);
    }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
