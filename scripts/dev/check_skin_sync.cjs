// 检查 SettingsExtensionsPanel 的皮肤选择是否跨窗口同步（storage 事件）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/webui/src/components/settings/SettingsExtensionsPanel.vue', 'utf8');
const idx = src.indexOf('function selectPetSkin');
console.log(src.slice(idx, idx + 300));
// 检查 skins.ts 的 setCurrentSkin 是否派发 storage 事件
const sk = fs.readFileSync('D:/Downloads/Marguerite/webui/src/components/pet/skins.ts', 'utf8');
console.log('--- setCurrentSkin ---');
const i2 = sk.indexOf('export function setCurrentSkin');
console.log(sk.slice(i2, i2 + 320));
