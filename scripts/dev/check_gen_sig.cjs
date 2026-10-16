// 看 generate_presentation 是否能拿到 EventBus / session（决定进度事件注入方式）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
const start = src.indexOf('fn generate_presentation');
console.log(src.slice(start - 100, start + 900));
