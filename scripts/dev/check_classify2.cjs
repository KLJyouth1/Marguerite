// 模拟 classify 完整打分：title="[架构设计] "
const rules = {
  Explain: { kws: ['explain','解释','这是什么','how does','what is'], w: 0.9 },
  Refactor: { kws: ['refactor','重构','优化','improve','clean up'], w: 0.9 },
  Fix: { kws: ['fix','修复','bug','错误','broken','not working'], w: 0.9 },
  Test: { kws: ['test','测试','单元测试','coverage'], w: 0.85 },
  Doc: { kws: ['doc','文档','document','注释','comment','方案','设计','规划','架构','实施步骤'], w: 0.9, pat: /(方案|设计|规划|架构|实施)/ },
  Commit: { kws: ['commit','提交','changelog','release notes'], w: 0.8 },
  Search: { kws: ['search','搜索','find','查找','where is'], w: 0.8 },
};
const title = '[架构设计] ';
const scores = {};
for (const [intent, r] of Object.entries(rules)) {
  const matched = r.kws.filter(k => title.includes(k)).length;
  let score = (matched / r.kws.length) * r.w;
  if (r.pat && r.pat.test(title)) score = Math.min(score + r.w * 0.3, r.w);
  if (score > 0) scores[intent] = score;
}
console.log('scores:', scores);
const best = Object.entries(scores).sort((a,b)=>b[1]-a[1])[0];
console.log('best:', best);
