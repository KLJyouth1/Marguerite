// 找 com.rs 的生成入口（可能是 pub unsafe fn 或宏内）与页循环
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
const fns = [...src.matchAll(/(pub )?(unsafe )?fn \w+/g)].map(m => ({ name: m[0], at: m.index }));
console.log('全部函数:');
fns.forEach(f => {
  const lineNo = src.slice(0, f.at).split('\n').length;
  console.log(`  L${lineNo}: ${f.name}`);
});
