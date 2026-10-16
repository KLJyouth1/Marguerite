// 检查 setCurrentSkin 是否派发 storage 事件（跨窗口同步的关键缺失）
const fs = require('fs');
const sk = fs.readFileSync('D:/Downloads/Marguerite/webui/src/components/pet/skins.ts', 'utf8');
console.log(sk.slice(sk.indexOf('export function setCurrentSkin')));
