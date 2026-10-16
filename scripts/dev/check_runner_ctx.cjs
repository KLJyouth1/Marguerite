// 检查 runner 里 title 变量在 classify 时的真实内容
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/plan/runner.rs', 'utf8');
// 找 progress 块附近上下文
const idx = src.indexOf('taskKind');
console.log(src.slice(idx - 700, idx + 200));
