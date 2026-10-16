// 审计四：数据结构对账——webui 读取的字段 vs 网关返回的真实结构
const WebSocket = require('ws');
const fs = require('fs');
const TOKEN_PATH = process.env.LOCALAPPDATA + '/Marguerite/gateway-token';
let token = ''; try { token = fs.readFileSync(TOKEN_PATH, 'utf8').trim(); } catch {}
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `s-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve(null), 8000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
  else if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
function keysOf(o) { return o && typeof o === 'object' ? Object.keys(o).sort() : []; }

async function run() {
  const out = {};
  // 1. onboarding.status：webui SetupStatus 接口期望的字段
  const st = (await rpc('onboarding.status')).payload || {};
  out.onboarding_status_keys = keysOf(st);
  const expectedOnboarding = ['needsOnboarding','hasConfig','llmConfigured','llmSource','channelCount','channelsConfigured','searchConfigured','imageGenerationConfigured','configPath','sections','sectionDetails','warnings','llmCredentialStatus','llmProfileStatus'];
  out.onboarding_missing = expectedOnboarding.filter(k => !(k in st));

  // 2. config.get：webui ConfigData 关键字段
  const cfg = (await rpc('config.get')).payload || {};
  const expectedCfg = ['model','llm','llm_profiles','llm_request_timeout_seconds','mcp','memory','skills','log','naming','privacy','guard','bloom'];
  out.config_missing = expectedCfg.filter(k => !(k in cfg));
  out.llm_keys = keysOf(cfg.llm);

  // 3. sessions.list 条目字段（webui SessionRow）
  const sl = (await rpc('sessions.list', { limit: 3 })).payload || {};
  const first = (sl.sessions || [])[0] || {};
  const expectedSession = ['key','title','summary','sessionKind','surface','messageCount','updatedAt','createdAt','status','runStatus','projectPath'];
  out.session_row_missing = expectedSession.filter(k => !(k in first));

  // 4. models.profiles.list 条目
  const mp = (await rpc('models.profiles.list')).payload || {};
  const p0 = (mp.profiles || [])[0] || {};
  out.profile_row_missing = ['id','label','provider','model','kind','builtin','active'].filter(k => !(k in p0));
  // webui 还期待 apiKeyConfigured/apiKeyEnv（档案卡凭证状态）
  out.profile_has_api_key_configured = 'apiKeyConfigured' in p0 || 'api_key_configured' in p0;

  // 5. usage.status 结构
  const us = await rpc('usage.status');
  out.usage_status_ok = !!(us && us.ok);

  // 6. plans.capabilities（EngineHealth 页）
  const pc = await rpc('plans.capabilities');
  out.plans_caps_ok = !!(pc && pc.ok);

  console.log(JSON.stringify(out, null, 2));
  process.exit(0);
}
