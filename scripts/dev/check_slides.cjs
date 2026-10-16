// 检查 com.rs 的 Slides.Add 循环是否发布事件（PPT 生成进度数据源判定）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
// 找 Slides.Add / for 循环
const idx = src.indexOf('Slides.Add');
console.log(src.slice(Math.max(0, idx - 600), idx + 400));
