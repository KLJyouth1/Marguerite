// 检查 research 深读模块（web_research_tool 用的 fetch+readability）的搜索引擎
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/tools/builtin.rs', 'utf8');
// 找 web_research_tool 的执行体里的搜索调用
const i = src.indexOf('fn web_research_tool');
const seg = src.slice(i, i + 3000);
const urls = [...seg.matchAll(/https?:\/\/[^\s"]+/g)].map(m => m[0]);
console.log('web_research_tool 内 URL:');
[...new Set(urls)].forEach(u => console.log(' ', u));
// 是否调用 run_web_search
console.log('调用 run_web_search:', seg.includes('run_web_search'));
