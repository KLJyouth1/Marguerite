// 查看 generate_presentation 的循环与 recorder 调用（进度事件注入点）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/desktop_automation/com.rs', 'utf8');
const seg = src.slice(src.indexOf('fn generate_presentation'), src.indexOf('fn generate_presentation') + 6000);
// 打印含 for / Recorder / slides_count 的行
seg.split('\n').forEach((line, i) => {
  if (/for |Recorder|slides|count|len\(\)|push|title/.test(line)) console.log(`L${i}: ${line.trim().slice(0, 100)}`);
});
