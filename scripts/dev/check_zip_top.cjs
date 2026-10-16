// zip 里到底打包了什么（顶层结构）
const { execSync } = require('child_process');
try {
  const out = execSync(
    `powershell -NoProfile -Command "Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::OpenRead('D:/Downloads/Marguerite/dist/marguerite-desktop-v0.2.5-chatfix-win64.zip').Entries | Select-Object -First 20 -ExpandProperty FullName"`,
    { shell: 'cmd.exe', timeout: 30000 }
  ).toString();
  console.log(out.trim());
} catch (e) { console.log('err', e.message.slice(0,100)); }
