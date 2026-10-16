// 检查 run_web_search 的完整引擎列表（可能还有第三回退：搜狗/百度）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/tools/builtin.rs', 'utf8');
// 提取 run_web_search 函数全文（2351-2410 行附近）
const lines = src.split('\n');
const seg = lines.slice(2350, 2460).join('\n');
// 找所有 http URL
const urls = [...seg.matchAll(/https?:\/\/[^\s"]+/g)].map(m => m[0]);
console.log('run_web_search 附近 URL:');
urls.forEach(u => console.log(' ', u));
