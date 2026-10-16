// 复杂任务复测：step_timeout=120s 后自动拆解应能走通
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0;
function chat(message, sessionKey, timeoutMs) {
  return new Promise((resolve) => {
    const onEvent = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'event' && m.event === 'session.event.done' && m.payload.session_key === sessionKey) {
        ws.off('message', onEvent); resolve({ ok: true, text: m.payload.text_snapshot || '' });
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent); resolve({ ok: false, error: JSON.stringify(m.payload).slice(0, 160) });
      }
    };
    ws.on('message', onEvent);
    ws.send(JSON.stringify({ type: 'req', id: 'k' + (++id), method: 'chat.send', params: { message, sessionKey } }));
    setTimeout(() => { ws.off('message', onEvent); resolve({ ok: false, error: 'TIMEOUT' }); }, timeoutMs);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const t5 = await chat(
      '帮我规划一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤',
      'ws.cx2', 300000
    );
    process.stdout.write(`[T5 复杂任务] ok=${t5.ok} 长度=${(t5.text || '').length}\n`);
    process.stdout.write('[预览] ' + (t5.text || t5.error || '').replace(/\n/g, ' ').slice(0, 180) + '\n');
    console.log('RETEST-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 330000);
