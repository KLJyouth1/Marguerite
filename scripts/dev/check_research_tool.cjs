// 检查 web_research_tool（2546 行的「检索/深读」）用什么引擎
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/tools/builtin.rs', 'utf8');
// 找 research 工具定义
const i = src.indexOf('fn web_research_tool');
console.log(src.slice(i, i + 800));
