// 找到占用 18790 的进程名（判断是否我们的后台网关）
const { execSync } = require('child_process');
try {
  const out = execSync('netstat -ano | findstr :18790', { shell: 'cmd.exe' }).toString();
  console.log(out.trim());
} catch (e) { console.log('无监听'); }
