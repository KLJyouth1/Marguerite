// 停止残留的旧网关进程（PID 84004，marguerite-core，占用 exe 导致重链接失败）
const { execSync } = require('child_process');
try {
  const out = execSync('taskkill /F /PID 84004', { shell: 'cmd.exe' }).toString();
  console.log(out.trim());
} catch (e) { console.log('kill 失败:', e.message); }
