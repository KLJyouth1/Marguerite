// 复现：reasoning 模型返回空 content + 长 reasoning_content 时，工具回路是否死循环
// DeepSeek v4 flash 在 tools + 大 max_tokens 下倾向先输出 reasoning_content，content 为空
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0; let thinkingCount = 0;
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: { message: '帮我规划一个完整的个人博客系统：文章、评论、标签的技术方案与实施步骤（分3步输出）', sessionKey: 'ws.diag4' } }));
  }
  if (m.type === 'event') {
    const dt = ((Date.now() - t0) / 1000).toFixed(0);
    if (m.event === 'session.event.thinking') { thinkingCount++; if (thinkingCount <= 5 || thinkingCount % 10 === 0) process.stdout.write(`[${dt}s] thinking #${thinkingCount}: ${JSON.stringify(m.payload).slice(0, 80)}\n`); }
    if (m.event === 'session.event.done') { process.stdout.write(`[${dt}s] DONE: ${JSON.stringify(m.payload).slice(0, 150)}\n`); console.log('总 thinking 帧:', thinkingCount); process.exit(0); }
    if (m.event === 'session.event.error') { process.stdout.write(`[${dt}s] ERROR: ${JSON.stringify(m.payload).slice(0, 160)}\n`); process.exit(0); }
    if (m.event === 'session.event.tool_call') process.stdout.write(`[${dt}s] tool_call\n`);
    if (m.event === 'session.event.tool_result') process.stdout.write(`[${dt}s] tool_result\n`);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT-240s, thinking frames:', thinkingCount); process.exit(1); }, 240000);
