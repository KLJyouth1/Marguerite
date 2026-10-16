// 办公任务 v2：逐项打印，workspace.file.* 先探测再执行
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
    const wf = await rpc('workspace.file.write', { path: 'qa_office_test.md', content: '# QA 办公测试\n\n- 项目评审会：周三 10:00\n' });
    process.stdout.write('[file.write] ' + (wf.timeout ? 'TIMEOUT' : 'ok=' + wf.ok + ' ' + JSON.stringify(wf.payload || wf.error).slice(0, 80)) + '\n');
    const rd = await rpc('workspace.file.read', { path: 'qa_office_test.md' });
    process.stdout.write('[file.read] ' + (rd.timeout ? 'TIMEOUT' : 'ok=' + rd.ok + ' has_content=' + /QA/.test(JSON.stringify(rd.payload || ''))) + '\n');
    const sc = await rpc('sessions.create', { title: '办公测试会话' });
    process.stdout.write('[sessions.create] ' + (sc.timeout ? 'TIMEOUT' : 'ok=' + sc.ok) + '\n');
    const sk = await rpc('skills.create', { name: 'qa-weekly-report', description: 'QA周报技能测试', content: '# 周报\n输出完成/进行中/风险三段。' });
    process.stdout.write('[skills.create] ' + (sk.timeout ? 'TIMEOUT' : JSON.stringify(sk.payload || sk.error).slice(0, 100)) + '\n');
    console.log('OFFICE-DONE');
    process.exit(0);
  }
});
setTimeout(() => { console.log('GLOBAL-TIMEOUT'); process.exit(1); }, 150000);
