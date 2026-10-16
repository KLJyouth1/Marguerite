// 复杂任务测试：自动拆解 + plans 落库 + ensemble/bloom 深度模式
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `c-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 30000);
  });
}
function chat(message, sessionKey, timeoutMs) {
  return new Promise((resolve) => {
    const onEvent = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'event' && m.event === 'session.event.done' && m.payload.session_key === sessionKey) {
        ws.off('message', onEvent); resolve(m.payload.text_snapshot || '');
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent); resolve('ERROR:' + JSON.stringify(m.payload).slice(0, 150));
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
  if (m.type === 'res' && m.id !== 'c' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
  if (m.type === 'res' && m.id === 'c') {
    // 复杂任务（触发自动拆解）
    const t5 = await chat('帮我规划一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤', 'ws.g5', 180000);
    process.stdout.write(`[T5 复杂任务] 长度=${t5.length} 有步骤=${/步骤|阶段|Step|1\.|2\./.test(t5)}\n`);
    process.stdout.write('[T5 预览] ' + t5.replace(/\n/g, ' ').slice(0, 150) + '\n');
    // plans 落库检查
    const plans = await rpc('plans.list');
    const runsN = ((plans.payload || {}).runs || []).length;
    const revsN = ((plans.payload || {}).plans || []).length;
    process.stdout.write(`[Plans 落库] revisions=${revsN} runs=${runsN}\n`);
    // Bloom 双模型并行（copilot 未配 → 应退化为同模型审校但仍走编排）
    await rpc('mode.switch', { kind: 'bloom' });
    const t6 = await chat('用两句话解释量子计算与经典计算的区别。', 'ws.g6', 120000);
    process.stdout.write(`[T6 Bloom] 长度=${t6.length} 回复=${t6.replace(/\n/g, ' ').slice(0, 100)}\n`);
    await rpc('mode.switch', { kind: 'agent' });
    console.log('COMPLEX-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 420000);
