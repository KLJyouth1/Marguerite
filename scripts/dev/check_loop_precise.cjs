// 打印 com.rs L96 循环前后精确内容（准备 patch）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
const start = src.indexOf('fn generate_presentation');
const seg = src.slice(start, start + 9000);
const lines = seg.split('\n');
lines.forEach((line, i) => {
  if (i >= 92 && i <= 102) console.log(`[rel L${i}] ${line}`);
});
