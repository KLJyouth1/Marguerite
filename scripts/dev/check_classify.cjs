// 模拟 classify('[架构设计] xxx')
const kw = ['doc', '文档', 'document', '注释', 'comment', '方案', '设计', '规划', '架构', '实施步骤'];
const title = '[架构设计] ';
const matched = kw.filter(k => title.toLowerCase().includes(k));
console.log('matched:', matched.length, '/', kw.length, '->', matched.join(','));
