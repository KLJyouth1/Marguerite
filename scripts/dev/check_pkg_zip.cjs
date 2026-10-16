// package_app.cmd 完整流程（zip 组装部分）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/scripts/package_app.cmd', 'utf8');
const lines = src.split('\n');
// 打印 zip / dist 相关段
lines.forEach((line, i) => {
  if (/zip|Compress|Archive|OUT|release/i.test(line)) console.log(`L${i+1}: ${line.trim().slice(0,110)}`);
});
