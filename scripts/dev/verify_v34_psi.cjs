// v3.4 PSI RPC 验证（修正：HelloOk features 在 res 顶层）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0;
function rpc(method, params) {
  return new Promise((resolve) => {
    const mid = 'v' + (++id);
    const h = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'res' && m.id === mid) { ws.off('message', h); clearTimeout(t); resolve(m); }
    };
    const t = setTimeout(() => { ws.off('message', h); resolve({ timeout: true }); }, 20000);
    ws.on('message', h);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const out = {};
    const st = await rpc('brain.status', {});
    out.status_timeout = !!st.timeout;
    out.brain_status_ok = !!(st.payload || {}).ok;
    const brain = (st.payload || {}).brain || {};
    out.phi = brain.phi_existence;
    out.mood = brain.dominant_mood;
    const cyc = await rpc('brain.psi_cycle', { input: '测试认知循环', hypotheses: ['方案甲', '方案乙'] });
    out.psi_cycle_ok = !!(cyc.payload || {}).ok;
    const night = await rpc('brain.set_circadian', { phase: 'night' });
    out.night_ok = !!(night.payload || {}).ok;
    await rpc('brain.set_circadian', { phase: 'day' });
    const it = await rpc('brain.interact', { positive: true });
    out.interact_mood = (it.payload || {}).mood;
    console.log(JSON.stringify(out, null, 2));
    console.log('PSI-RPC:', (out.brain_status_ok && out.psi_cycle_ok && out.night_ok) ? 'PASS' : 'FAIL');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 90000);
