// 复现子步骤 dispatch 挂起：直接调 plans.implement 用已存在的 revision
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params) {
  return new Promise((resolve) => {
    const mid = 'i' + (++id); pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 25000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    // 列出已有 revisions，取最新的 current
    const plans = await rpc('plans.list', {});
    const revs = ((plans.payload || {}).plans || []).filter(r => r.current);
    process.stdout.write('current revisions: ' + revs.length + '\n');
    if (!revs.length) { console.log('无可用修订'); process.exit(1); }
    const rid = revs[0].revisionId;
    process.stdout.write('用 revision: ' + rid + ' steps=' + JSON.stringify((revs[0].steps||[]).map(s=>s.stepId)) + '\n');
    // 启动运行
    const run = await rpc('plans.run', { revisionId: rid });
    const runId = run.payload && run.payload.run && run.payload.run.runId;
    process.stdout.write('runId=' + runId + '\n');
    if (!runId) process.exit(1);
    // 现在手动执行一步（模拟 runner 的单步）：直接观察 chat_with_tools 卡点
    // 改为：等待 150s 后检查 run 状态是否推进
    let lastStatus = '';
    for (let i = 0; i < 15; i++) {
      await new Promise(r => setTimeout(r, 10000));
      const chk = await rpc('plans.checklist', { runId });
      const st = JSON.stringify(chk.payload && chk.payload.run && chk.payload.run.steps || []).slice(0, 200);
      if (st !== lastStatus) { process.stdout.write(`[${(i + 1) * 10}s] ${st}\n`); lastStatus = st; }
    }
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 180000);
