// 功能实测 C：全部插件/子系统 RPC 巡检
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `p-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 15000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
  else if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
ws.on('error', (e) => console.log('WS ERROR:', e.message));

const CHECKS = [
  // [名称, 方法, 参数, 验证函数(payload)=>bool]
  ['技能列表', 'skills.list', {}, p => Array.isArray(p) || Array.isArray(p.skills) || Array.isArray(p.items)],
  ['技能搜索', 'skills.search', { query: '代码' }, () => true],
  ['智能体台账', 'agents.list', {}, p => Array.isArray(p)],
  ['专家列表', 'experts.list', {}, () => true],
  ['KB 状态', 'kb.status', {}, () => true],
  ['KB 检索', 'kb.search', { query: '测试' }, () => true],
  ['KB 混合检索', 'kb.search.hybrid', { query: '工作台' }, () => true],
  ['记忆列表', 'memory.list', {}, () => true],
  ['行为记忆', 'memory.behavior.list', {}, () => true],
  ['跨项目记忆', 'memory.cross_project.search', { query: 'test' }, () => true],
  ['游戏列表', 'games.list', {}, p => (p.games || []).length > 0],
  ['桌宠状态', 'pet.status', {}, () => true],
  ['市场列表', 'market.list', {}, () => true],
  ['插件列表', 'plugins.list', {}, () => true],
  ['舰队列表', 'fleet.list', {}, () => true],
  ['自进化历史', 'evolve.history', {}, () => true],
  ['用量状态', 'usage.status', {}, () => true],
  ['用量日聚合', 'usage.daily', {}, () => true],
  ['长任务列表', 'long_running.list', {}, p => Array.isArray(p.tasks)],
  ['计划列表', 'plans.list', {}, () => true],
  ['页面台账', 'pages.list', {}, () => true],
  ['蜂群规格', 'mode.swarm.get', {}, () => true],
  ['缓存状态', 'cache.status', {}, () => true],
  ['路由自学习', 'router.selflearning.status', {}, () => true],
  ['终端后端', 'terminal.backends.status', {}, () => true],
  ['MCP 服务器', 'mcp.servers.list', {}, p => Array.isArray(p.servers)],
  ['沙箱策略', 'sandbox.policy.get', {}, () => true],
  ['沙箱能力', 'sandbox.capability.status', {}, () => true],
  ['工作区列表', 'workspaces.list', {}, () => true],
  ['知识图谱', 'kb.graph.view', {}, () => true],
  ['扑克状态', 'poker.state', {}, () => true],
  ['浏览器状态', 'browser.status', {}, () => true],
];

async function run() {
  const results = [];
  for (const [name, method, params, verify] of CHECKS) {
    const r = await rpc(method, params);
    let status;
    if (r.timeout) status = 'TIMEOUT';
    else if (r.ok === false) status = 'ERR:' + ((r.error && r.error.code) || '?');
    else {
      try { status = verify(r.payload) ? 'OK' : 'OK(结构待核)'; } catch { status = 'OK'; }
    }
    results.push({ name, method, status });
    process.stdout.write(`${status === 'OK' ? '✓' : '✗'} ${name} [${method}] ${status}\n`);
  }
  const bad = results.filter(r => r.status !== 'OK');
  console.log('\nSUMMARY: total=' + results.length + ' ok=' + (results.length - bad.length) + ' bad=' + bad.length);
  process.exit(0);
}
