// 复杂任务复测（step_timeout 120s 后）+ 新模型矩阵验证
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `r-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 30000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
  else if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
async function run() {
  const out = {};
  // 1. 模型矩阵迁移（seed_revision 2→3 应自动补齐新档案）
  const mp = (await rpc('models.profiles.list', {})).payload || {};
  const ids = (mp.profiles || []).map(p => p.id);
  out.total_profiles = ids.length;
  out.matrix = {};
  for (const expect of ['dashscope-qwen-max', 'dashscope-qwen-vl-max', 'zhipu-glm-4-plus', 'zhipu-glm-4v-plus', 'moonshot-kimi-k2', 'moonshot-kimi-vision', 'gemini-2-5-flash', 'gemini-2-5-pro', 'openai-o4-mini', 'anthropic-claude-haiku-4']) {
    out.matrix[expect] = ids.includes(expect) ? 'OK' : 'MISSING';
  }
  out.active_still = mp.active;
  console.log(JSON.stringify(out, null, 2));
  process.exit(0);
}
