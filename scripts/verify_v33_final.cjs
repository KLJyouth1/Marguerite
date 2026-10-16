// v3.3 最终验证：插话 + 审批事件 + 步骤进度 三链路
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let taskId = ''; let t0 = 0;
const out = {
  steer_ok: false, steer_accepted_event: false, steers_applied_event: false,
  step_progress_seen: false, task_kind_badge: '', injected_in_reply: false,
  methods_has_steer: false, methods_has_progress: true,
};
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const feats = (m.payload && m.payload.features) || m.features || { methods: [] };
    out.methods_has_steer = feats.methods.includes('sessions.steer.v2');
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '帮我开发一个完整的个人博客系统：文章发布、评论、标签分类的技术方案与实施步骤',
      sessionKey: 'ws.v33.final',
    }}));
    setTimeout(() => {
      if (!taskId) return;
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.v33.final',
        message: '补充要求：优先考虑国内可访问的服务商',
        expected_turn_id: taskId,
        client_request_id: 'cr-fin-' + Date.now(),
        client_message_id: 'cm-fin-' + Date.now(),
      }}));
    }, 45000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'res' && m.id === 'st') out.steer_ok = !!(m.payload && m.payload.accepted);
  if (m.type === 'event') {
    const ev = m.event || '';
    if (ev.includes('progress') || (m.payload && m.payload.kind === 'step_progress')) {
      const p = m.payload || {};
      if (p.kind === 'step_progress') {
        out.step_progress_seen = true;
        out.task_kind_badge = p.taskKind || '';
      }
    }
    if (ev.includes('status')) {
      if ((m.payload || {}).kind === 'steer_accepted') out.steer_accepted_event = true;
      if ((m.payload || {}).kind === 'steers_applied') out.steers_applied_event = true;
    }
    if (ev === 'session.event.done') {
      out.injected_in_reply = /用户插话/.test(m.payload.text_snapshot || '') && /国内可访问/.test(m.payload.text_snapshot || '');
      console.log(JSON.stringify(out, null, 2));
      const pass = out.steer_ok && out.steer_accepted_event && out.steers_applied_event
        && out.step_progress_seen && out.injected_in_reply && out.methods_has_steer && !!out.task_kind_badge;
      console.log('V33-FINAL:', pass ? 'ALL-PASS' : 'PARTIAL');
      process.exit(pass ? 0 : 1);
    }
  }
});
setTimeout(() => { console.log('TIMEOUT'); console.log(JSON.stringify(out, null, 2)); process.exit(1); }, 420000);
