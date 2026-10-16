// 办公任务 v3：skills.create 用正确字段 prompt_template + 游戏对战 + 桌宠互动
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0;
function rpc(method, params) {
  return new Promise((resolve) => {
    const mid = 'o' + (++id);
    const onMsg = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'res' && m.id === mid) { ws.off('message', onMsg); clearTimeout(t); resolve(m); }
    };
    const t = setTimeout(() => { ws.off('message', onMsg); resolve({ timeout: true }); }, 30000);
    ws.on('message', onMsg);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const out = {};
    const sk = await rpc('skills.create', {
      name: 'qa-weekly-report',
      description: 'QA周报技能测试',
      prompt_template: '# 周报生成器\n根据本周对话记录输出：本周期完成事项、进行中事项、风险与阻塞三段。'
    });
    out.skill_create_ok = !!(sk.payload && sk.payload.success !== false && sk.payload.ok !== false);
    // 游戏：井字棋完整对局
    let g = await rpc('games.play', { game: 'tictactoe', moves: ['0', '4', '1', '7', '2'] });
    out.tictactoe_win = !!(g.payload && g.payload.ok) && /获胜|win/i.test(JSON.stringify(g.payload));
    // 扑克创建
    const pk = await rpc('poker.create', {});
    out.poker_create_ok = !!(pk.payload && (pk.payload.roomId || pk.payload.id || pk.payload.code));
    // 桌宠动作
    const pa = await rpc('pet.action', { action: 'feed' });
    out.pet_feed_ok = !(pa.payload && pa.payload.ok === false);
    // KB 灌入
    const ki = await rpc('kb.ingest', { title: 'qa-doc', text: 'Marguerite 是一个本地优先的 AI Agent 工作台，支持多模型路由。' });
    out.kb_ingest_ok = !(ki.payload && ki.payload.ok === false);
    console.log(JSON.stringify(out, null, 2));
    console.log('OFFICE2-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 150000);
