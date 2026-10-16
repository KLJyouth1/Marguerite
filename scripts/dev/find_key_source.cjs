// 探查 key 的真实来源：credentials 台账 / models.json 之外的位置
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
ws.on('open', () => ws.send(JSON.stringify({type:'req',id:'c',method:'connect',params:{auth:{token},protocol:{min:3,max:3}}})));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') run();
});
async function run() {
  // credentials store 常见位置
  const fs2 = require('fs');
  const path = require('path');
  const dataDir = process.env.LOCALAPPDATA + '/Marguerite';
  const candidates = ['credentials.json','credentials.db','secrets.json','byok.json'];
  for (const c of candidates) {
    const p = path.join(dataDir, c);
    if (fs2.existsSync(p)) console.log('发现:', p, fs2.statSync(p).size, 'bytes');
  }
  // doctor 看 key 来源
  ws.send(JSON.stringify({type:'req',id:'d',method:'doctor.status',params:{}}));
  ws.on('message', function h(raw) {
    const m = JSON.parse(raw.toString());
    if (m.type === 'res' && m.id === 'd') {
      const chk = (m.payload.checks||[]).find(x=>x.name==='model');
      console.log('doctor model 检查:', JSON.stringify(chk));
      process.exit(0);
    }
  });
}
setTimeout(()=>{console.log('timeout');process.exit(1)}, 8000);
