// run_web_search 的引擎选择：DDG 失败时回退什么？（搜狗结果从哪来）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/tools/builtin.rs', 'utf8');
// 搜狗不在 builtin.rs。查 research 模块
const files = ['core/src/research/mod.rs','core/src/research/engine.rs','core/src/tools/research.rs'];
for (const f of files) {
  try {
    const s = fs.readFileSync('D:/Downloads/Marguerite/marguerite/' + f, 'utf8');
    if (s.includes('sogou')) { console.log(f, '含 sogou'); }
  } catch {}
}
// 全局搜
console.log('--- 全局 grep sogou ---');
