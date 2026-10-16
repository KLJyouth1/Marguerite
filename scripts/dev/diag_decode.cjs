// 定位 decode 错误：直接用长 prompt 打 chat.send（非拆解路径）看是否复现
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
        ws.off('message', onEvent); resolve('OK:' + (m.payload.text_snapshot || '').slice(0, 60));
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent); resolve('ERR:' + JSON.stringify(m.payload.message || m.payload).slice(0, 130));
      }
    };
    ws.on('message', onEvent);
    ws.send(JSON.stringify({ type: 'req', id: 'k' + (++id), method: 'chat.send', params: { message, sessionKey } }));
    setTimeout(() => { ws.off('message', onEvent); resolve('TIMEOUT'); }, timeoutMs);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    // 同一复杂任务，但明确说「只做规划」避免触发 auto_decompose（走普通单轮路径）
    const r1 = await chat('请为个人博客系统（文章发布、评论、标签分类）写一份简明技术方案，300字以内。这是纯咨询，不需要执行。', 'ws.dg1', 150000);
    process.stdout.write('[单轮路径] ' + r1.slice(0, 160) + '\n');
    console.log('DIAG-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 180000);
