// 检查 recorder 的步骤事件是否已发布到总线（PPT 生成过程可视化数据源）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/recorder.rs', 'utf8');
console.log('=== recorder.rs 头部 ===');
console.log(src.slice(0, 1200));
