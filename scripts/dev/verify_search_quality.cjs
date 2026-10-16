// 复现「简单联网搜索任务」质量：搜一个实时性问题
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0; let toolCalls = [];
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '搜索一下今天的科技新闻，给我三条摘要',
      sessionKey: 'ws.search.qa',
    }}));
    return;
  }
  if (m.type === 'event') {
    const ev = (m.event || '').replace('session.event.', '');
    const dt = ((Date.now() - t0) / 1000).toFixed(0);
    if (ev === 'tool_call') {
      const p = m.payload || {};
      toolCalls.push(String(p.tool || p.name || '?'));
      console.log(`[${dt}s] tool_call: ${p.tool || p.name}`);
    }
    if (ev === 'tool_result' && m.payload) {
      console.log(`[${dt}s] tool_result ok=${m.payload.ok} len=${JSON.stringify(m.payload).length}`);
    }
    if (ev === 'done') {
      const text = m.payload.text_snapshot || '';
      console.log('=== done ===');
      console.log('工具调用序列:', JSON.stringify(toolCalls));
      console.log('回复长度:', text.length);
      console.log('--- 回复前 600 字 ---');
      console.log(text.slice(0, 600));
      process.exit(0);
    }
    if (ev === 'error') { console.log('ERR:', JSON.stringify(m.payload).slice(0, 150)); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 300000);
