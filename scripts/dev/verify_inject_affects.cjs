// v3.3.1 核心验证：插话影响生成本身（而非仅附录）
// 发起长回复任务 → 运行中插入「用红色主题」→ 检查回复正文是否体现
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
let taskId = ''; let t0 = 0;
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    t0 = Date.now();
    ws.send(JSON.stringify({ type: 'req', id: 't', method: 'chat.send', params: {
      message: '帮我开发一个完整的个人博客系统：文章发布、评论、标签分类三大功能的技术方案与实施步骤',
      sessionKey: 'ws.inject.v2',
    }}));
    // 计划路径启动后（约 25s）注入纠正——此时 step-1 已开始，lead 在工具循环边界装配消息时会带上
    setTimeout(() => {
      if (!taskId) return;
      ws.send(JSON.stringify({ type: 'req', id: 'st', method: 'sessions.steer.v2', params: {
        key: 'ws.inject.v2',
        message: '重要补充：技术选型必须优先考虑国内可访问的服务商（如阿里云），并在方案中明确说明',
        expected_turn_id: taskId,
        client_request_id: 'cr-inj-' + Date.now(),
        client_message_id: 'cm-inj-' + Date.now(),
      }}));
      console.log('[steer] 已发送');
    }, 30000);
    return;
  }
  if (m.type === 'res' && m.id === 't') taskId = (m.payload || {}).taskId || '';
  if (m.type === 'event') {
    const ev = m.event || '';
    if (ev === 'session.event.done') {
      const text = m.payload.text_snapshot || '';
      const hasAppendix = /运行期间收到你的补充/.test(text);
      const inBody = /阿里云/.test(text.replace(/[\s\S]*运行期间收到你的补充[\s\S]*/, ''));
      console.log('done 长度:', text.length);
      console.log('含附录区块:', hasAppendix);
      console.log('正文中体现「阿里云」(非附录区):', inBody);
      console.log('判定:', (hasAppendix && inBody) ? 'INJECT-AFFECTS-GENERATION ✓' : (hasAppendix ? '仅附录模式' : 'FAIL'));
      process.exit(0);
    }
    if (ev === 'session.event.error') { console.log('ERR:', JSON.stringify(m.payload).slice(0,150)); process.exit(1); }
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 420000);
