// 检查 file_change::global_bus 是否可在 com.rs 中复用（进度事件通道）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/tools/file_change.rs', 'utf8');
const idx = src.indexOf('fn global_bus');
console.log(src.slice(idx - 200, idx + 400));
