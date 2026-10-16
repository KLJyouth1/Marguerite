// 打印 webui_dir 完整函数（打包版从哪找 webui/dist）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/gui/src-tauri/src/lib.rs', 'utf8');
const i = src.indexOf('fn webui_dir');
console.log(src.slice(i - 300, i + 1500));
