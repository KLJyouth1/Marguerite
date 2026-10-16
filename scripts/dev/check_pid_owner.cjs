// 检查 PID 84004 是否 marguerite-core（是→可安全停止；否则不动）
const { execSync } = require('child_process');
try {
  const out = execSync('tasklist /FI "PID eq 84004" /FO CSV', { shell: 'cmd.exe' }).toString();
  const isCore = out.toLowerCase().includes('marguerite-core');
  console.log('is_marguerite_core:', isCore);
  console.log(out.split('\n').slice(1).join('\n').trim());
} catch (e) { console.log('查询失败', e.message); }
