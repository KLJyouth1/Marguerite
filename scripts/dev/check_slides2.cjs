// 找 Slides.Add 实际调用点（第二次出现，非文档注释）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
let pos = -1;
for (let i = 0; i < 3; i++) {
  pos = src.indexOf('Slides.Add', pos + 1);
  if (pos === -1) break;
  const lineStart = src.lastIndexOf('\n', pos);
  const line = src.slice(lineStart + 1, src.indexOf('\n', pos)).trim();
  console.log(`#${i + 1}:`, line.slice(0, 90));
}
