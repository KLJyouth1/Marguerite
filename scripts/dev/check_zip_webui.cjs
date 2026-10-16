// 确认 zip 里 webui/dist 结构（pets 缺失范围）
const { execSync } = require('child_process');
try {
  const out = execSync(
    `powershell -NoProfile -Command "Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::OpenRead('D:/Downloads/Marguerite/dist/marguerite-desktop-v0.2.5-chatfix-win64.zip').Entries | Where-Object { $_.FullName -match 'webui' } | Select-Object -First 15 -ExpandProperty FullName"`,
    { shell: 'cmd.exe', timeout: 30000 }
  ).toString();
  console.log(out.trim() || '(无 webui 条目)');
} catch (e) { console.log('err', e.message.slice(0,100)); }
