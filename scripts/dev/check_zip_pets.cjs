// 核对打包产物 zip 内是否含 pets（用户拿到的安装包）
const { execSync } = require('child_process');
try {
  // 用 powershell 列 zip 内容
  const out = execSync(
    `powershell -NoProfile -Command "Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::OpenRead('D:/Downloads/Marguerite/dist/marguerite-desktop-v0.2.5-chatfix-win64.zip').Entries | Where-Object { $_.FullName -match 'pets' } | Select-Object -First 8 -ExpandProperty FullName"`,
    { shell: 'cmd.exe', timeout: 30000 }
  ).toString();
  console.log('v0.2.5 包内 pets 条目:\n' + (out.trim() || '(无) ← 打包缺图确认'));
} catch (e) { console.log('读取失败:', e.message.slice(0, 150)); }
