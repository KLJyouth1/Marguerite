// 审计产物生成路径：哪些工具产出产物、事件负载里有什么
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
const idx = src.indexOf('record_artifact_with_session');
console.log('--- com.rs 产物登记上下文 ---');
console.log(src.slice(Math.max(0, idx - 400), idx + 120));
