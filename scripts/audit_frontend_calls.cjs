// 审计三：前端实际调用的全部 RPC 方法 → 网关逐一探测
const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');
const TOKEN_PATH = process.env.LOCALAPPDATA + '/Marguerite/gateway-token';
let token = ''; try { token = fs.readFileSync(TOKEN_PATH, 'utf8').trim(); } catch {}

const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `w-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true, _t: true }), 6000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
  else if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
ws.on('error', () => {});

const FRONTEND_METHODS = [
  // 设置页
  'onboarding.catalog', 'onboarding.status', 'onboarding.router.configure', 'onboarding.ensemble.configure',
  'onboarding.provider.configure', 'onboarding.provider.probe', 'onboarding.llmProfile.upsert',
  'onboarding.llmProfile.activate', 'onboarding.llmProfile.active.remove', 'config.get', 'config.patch.safe', 'config.patch',
  'config.effective', 'models.profiles.list', 'models.profiles.set_active', 'models.profiles.set_copilot',
  'models.profiles.add', 'models.profiles.remove', 'models.list', 'models.routing.get', 'models.routing.set',
  'sandbox.policy.get', 'sandbox.policy.update', 'sandbox.policy.defaults', 'sandbox.status',
  'memory.layers.status', 'memory.list', 'memory.behavior.list', 'memory.behavior.summary',
  'plans.capabilities', 'doctor.status', 'system.status',
  // 工作台
  'sessions.list', 'sessions.create', 'sessions.delete', 'sessions.get',
  'chat.history', 'tools.catalog', 'usage.status', 'usage.query', 'usage.daily',
  'skills.list', 'skills.search', 'skills.reload', 'agents.list', 'experts.list',
  'kb.status', 'kb.search', 'kb.search.hybrid', 'kb.list', 'graph.project.list', 'graph.project.view',
  'market.list', 'market.search', 'plugins.list', 'evolve.history', 'fleet.list', 'runtime.status',
  'mode.get', 'mode.spec', 'mode.swarm.get', 'cache.status', 'router.decision', 'router.selflearning.status',
  'pet.status', 'games.list', 'long_running.list', 'workspaces.list', 'pages.list',
  'terminal.backends.status', 'mcp.servers.list', 'mcp.list', 'context.status',
];

async function run() {
  const broken = [], okCount = [], slow = [];
  for (const m of FRONTEND_METHODS) {
    const r = await rpc(m, {});
    if (r._t) { slow.push(m); continue; }
    if (r.ok === false) {
      const code = r.error && r.error.code;
      if (code === 'NOT_IMPLEMENTED') broken.push(m);
      // 其他业务错误（如缺参数 INVALID_REQUEST）说明方法本身可达，不算断链
    } else { okCount.push(m); }
  }
  console.log(JSON.stringify({
    probed: FRONTEND_METHODS.length,
    reachable_ok: okCount.length,
    business_error_but_reachable: FRONTEND_METHODS.length - okCount.length - broken.length - slow.length,
    NOT_IMPLEMENTED: broken,
    timeout: slow,
  }, null, 2));
  process.exit(0);
}
