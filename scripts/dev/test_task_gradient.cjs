// 功能实测 B：任务梯度（复杂/日常/办公）真实 LLM 执行 + 工具链路
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0;
function chat(message, sessionKey, timeoutMs = 120000) {
  return new Promise((resolve) => {
    const onEvent = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'event' && m.event === 'session.event.done' && m.payload.session_key === sessionKey) {
        ws.off('message', onEvent);
        resolve({ ok: true, text: (m.payload.text_snapshot || m.payload.text || ''), reason: m.payload.reason });
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent);
        resolve({ ok: false, error: JSON.stringify(m.payload).slice(0, 180) });
      }
    };
    ws.on('message', onEvent);
    ws.send(JSON.stringify({ type: 'req', id: `chat-${++id}`, method: 'chat.send', params: { message, sessionKey } }));
    setTimeout(() => { ws.off('message', onEvent); resolve({ ok: false, error: `timeout-${timeoutMs / 1000}s` }); }, timeoutMs);
  });
}
function rpc(method, params = {}) {
  return new Promise((resolve) => {
    const mid = `b-${++id}`; pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 20000);
  });
}
const pending = new Map();
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); p(m); }
});
ws.on('error', (e) => console.log('WS ERROR:', e.message));

async function run() {
  const out = {};
  // 确保回到 agent 模式
  await rpc('mode.switch', { kind: 'agent' });

  // ── 简单日常 ──
  let r = await chat('把这句话翻译成英文：今天天气很好。只输出译文。', 'ws.t.simple');
  out.simple_translate = { ok: r.ok, reply: (r.text || '').slice(0, 80) };

  // ── 日常问答（数学推理）──
  r = await chat('一个班有30人，其中5/6是女生，女生比男生多几人？给出计算过程。', 'ws.t.math');
  out.math_reasoning = { ok: r.ok, has_process: /20|10|25|5/.test(r.text || ''), reply: (r.text || '').replace(/\n/g, ' ').slice(0, 100) };

  // ── 办公任务（结构化输出）──
  r = await chat('为「周三上午10点项目评审会」写一封中文会议通知邮件，包含主题/时间/地点/议程四项。用简洁格式。', 'ws.t.office');
  out.office_email = { ok: r.ok, has_structure: /主题|时间|地点|议程/.test(r.text || ''), reply_len: (r.text || '').length };

  // ── 办公任务（JSON 输出）──
  r = await chat('输出严格 JSON：{"tasks":[{"name":"写周报","priority":"高"}]} 格式，列3个本周工作任务。只输出 JSON。', 'ws.t.json');
  let jsonOk = false;
  try { const j = JSON.parse((r.text || '').match(/\{[\s\S]*\}/)?.[0] || ''); jsonOk = Array.isArray(j.tasks) && j.tasks.length >= 3; } catch {}
  out.json_output = { ok: r.ok, valid_json_with_3_tasks: jsonOk };

  // ── 中等复杂（代码生成）──
  r = await chat('用 Python 写一个函数判断字符串是否是回文，忽略大小写和空格。附2行测试。', 'ws.t.code');
  out.code_gen = { ok: r.ok, has_code: /def\s+\w+|::/.test(r.text || '') || /def /.test(r.text || ''), reply_len: (r.text || '').length };

  // ── 复杂任务（自动拆解触发）──
  r = await chat('帮我设计并实现一个完整的个人博客系统，要求有文章发布、评论、标签分类功能', 'ws.t.complex', 150000);
  out.complex_decompose = { ok: r.ok, reply_len: (r.text || '').length, preview: (r.text || '').replace(/\n/g, ' ').slice(0, 120) };

  // plans 是否落库
  const plans = await rpc('plans.list');
  out.plans_created = ((plans.payload || {}).runs || []).length > 0 || ((plans.payload || {}).plans || []).length > 0;

  console.log(JSON.stringify(out, null, 2));
  process.exit(0);
}
