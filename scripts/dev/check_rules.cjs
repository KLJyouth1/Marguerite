// Question 规则检查
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/intent/classifier.rs', 'utf8');
console.log('Question 出现次数:', (src.match(/Question/g) || []).length);
// 查找所有规则 intent 行
const intents = src.match(/intent: IntentType::\w+/g);
console.log('规则列表:', JSON.stringify(intents));
