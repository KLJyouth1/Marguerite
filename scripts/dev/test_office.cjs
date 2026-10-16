// 办公任务实测：文件写入（沙箱内）+ 会话管理
const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');
const os = require('os');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:18790/ws');
let id = 0; const pending = new Map();
function rpc(method, params) {
  return new Promise((resolve) => {
    const mid = 'o' + (++id); pending.set(mid, resolve);
    ws.send(JSON.stringify({ type: 'req', id: mid, method, params }));
    setTimeout(() => resolve({ timeout: true }), 60000);
  });
}
ws.on('open', () => ws.send(JSON.stringify({ type: 'req', id: 'c', method: 'connect', params: { auth: { token }, protocol: { min: 3, max: 3 } } })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.type === 'res' && m.id === 'c') {
    const out = {};
    // 1. 沙箱文件写入
    const wf = await rpc('workspace.file.write', {
      path: 'qa_office_test.md',
      content: '# QA 办公测试\n\n- 项目评审会：周三 10:00\n- 输出：会议纪要模板\n'
    });
    out.file_write_ok = !!(wf && wf.ok);
    // 2. 读回验证
    const rd = await rpc('workspace.file.read', { path: 'qa_office_test.md' });
    out.file_read_back_ok = !!(rd && rd.ok) && /QA 办公测试/.test(JSON.stringify(rd.payload || ''));
    // 3. 会话创建+删除
    const sc = await rpc('sessions.create', { title: '办公测试会话' });
    const newKey = sc.payload && sc.payload.session && sc.payload.session.key;
    out.session_create_ok = !!(sc && sc.ok);
    const sd = await rpc('sessions.delete', { key: newKey });
    out.session_delete_ok = !!(sd && sd.ok);
    // 4. 技能创建（办公自动化技能）
    const sk = await rpc('skills.create', {
      name: 'weekly-report',
      description: '生成周报',
      content: '# 周报生成器\n根据本周对话记录输出：本周期完成事项、进行中事项、风险与阻塞三段。'
    });
    out.skill_create = sk && sk.ok !== false;
    console.log(JSON.stringify(out, null, 2));
    process.exit(0);
  }
});
setTimeout(() => { console.log('TIMEOUT'); process.exit(1); }, 90000);
