// 检查注入路径：单轮（非计划）任务里 steer 是否进入 lead 装配
// 单轮路径 run_turn_reply → RealEngine.execute_turn → steer_provider drain → forward
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let t0 = 0;
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    // 中等长度任务：不触发自动拆解，走单轮 RealEngine 路径
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '写一篇 300 字的短文介绍一个你选择的城市',
      sessionKey: 'ws.inject.single',
    }}));
    // 单轮回复较快，3 秒后插话（此时 LLM 请求已发出——本轮来不及，
    // 但若工具循环多轮则下一轮装配会带上）
    setTimeout(() => {
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.single',
        message: '改成介绍成都，并重点提到美食',
        expected_turn_id: (m.payload || {}).taskId || 'turn-x',
        client_request_id: 'cr-si-' + Date.now(),
        client_message_id: 'cm-si-' + Date.now(),
      }}));
    }, 3000);
    return;
  }
  if (m.type === 'event') {
    if (m.event === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      console.log('done 长度:', text.length);
      console.log('内容:', text.slice(0, 120));
      console.log('提及成都:', /成都/.test(text));
      process.exit(0);
    }
    if (m.event === 'session.event.error') { console.log('ERR'); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 180000);
