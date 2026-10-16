// 渐进式任务测试 v2：每个任务独立打印（避免 stdout 缓冲吞输出）
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0;
function chat(message, sessionKey, timeoutMs) {
  return new Promise((resolve) => {
    const onEvent = (raw) => {
      const m = JSON.parse(raw.toString());
      if (m.type === 'event' && m.event === 'session.event.done' && m.payload.session_key === sessionKey) {
        ws.off('message', onEvent); resolve(m.payload.text_snapshot || '');
      }
      if (m.type === 'event' && m.event === 'session.event.error' && (m.payload.session_key || m.payload.key) === sessionKey) {
        ws.off('message', onEvent); resolve('ERROR:' + JSON.stringify(m.payload).slice(0, 120));
      }
    };
    ws.on('message', onEvent);
    ws.send(JSON.stringify({ type: 'req', id: 'k' + (++id), method: 'chat.send', params: { message, sessionKey } }));
    setTimeout(() => { ws.off('message', onEvent); resolve('TIMEOUT'); }, timeoutMs);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const t1 = await chat('把这句话翻译成英文：今天天气很好。只输出译文。', 'ws.g1', 60000);
    process.stdout.write('[T1 翻译] ' + t1.slice(0, 80) + '\n');
    const t2 = await chat('一个班30人，5/6是女生，女生比男生多几人？给出计算过程。', 'ws.g2', 90000);
    process.stdout.write('[T2 数学] ' + t2.replace(/\n/g, ' ').slice(0, 110) + '\n');
    const t3 = await chat('为周三上午10点项目评审会写一封中文会议通知邮件，包含主题/时间/地点/议程四项。', 'ws.g3', 90000);
    const ok3 = /主题|时间|地点|议程/.test(t3);
    process.stdout.write(`[T3 邮件] 结构完整=${ok3} 长度=${t3.length}\n`);
    const t4 = await chat('用 Python 写一个函数判断回文（忽略大小写和空格），附测试。', 'ws.g4', 90000);
    process.stdout.write(`[T4 代码] 有函数=${/def /.test(t4)} 长度=${t4.length}\n`);
    console.log('GRADIENT-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 360000);
