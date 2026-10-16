// 复现 long_running.list 偶发挂起：并发 + 交错调用
const WebSocket = require('ws');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
const t0 = Date.now();
ws.on('open', () => ws.send(JSON.stringify({type:'req',id:'c',method:'connect',params:{auth:{token},protocol:{min:3,max:3}}})));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    // 顺序 10 次，每次间隔 50ms —— 模拟前端轮询
    let i = 0;
    const timer = setInterval(() => {
      i++;
      if (i > 10) { clearInterval(timer); console.log('全部完成，无挂起'); process.exit(0); }
      const id = 'lr' + i;
      const s = Date.now();
      const h = (mm) => {
        if (mm.type === 'res' && mm.id === id) {
          const dt = Date.now() - s;
          if (dt > 1000) console.log(`call ${i}: ${dt}ms ← 慢`);
          ws.off('message', h);
        }
      };
      ws.on('message', h);
      setTimeout(() => { console.log(`call ${i} 超过5秒未响应！`); process.exit(1); }, 5000);
      ws.send(JSON.stringify({type:'req',id,method:'long_running.list',params:{}}));
    }, 50);
  }
});
setTimeout(() => process.exit(0), 8000);
