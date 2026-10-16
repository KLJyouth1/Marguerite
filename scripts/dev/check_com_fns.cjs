// 检查 com.rs 的 generate 函数签名与循环结构（找进度事件注入点）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
// 找 pub fn
const fns = [...src.matchAll(/pub fn \w+\([^)]*\)/g)].map(m => m[0]);
console.log('公开函数:');
fns.forEach(f => console.log(' ', f.slice(0, 80)));
// 找 for 循环
const loops = [...src.matchAll(/for \w+ in [^\n{]+/g)].map(m => m[0]);
console.log('循环:');
loops.forEach(l => console.log(' ', l.slice(0, 90)));
