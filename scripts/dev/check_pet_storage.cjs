// 检查 DesktopPet 的 storage 监听（跨窗口同步路径）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/webui/src/components/pet/DesktopPet.vue', 'utf8');
const i = src.indexOf("event.key === 'pet.skin'");
console.log(src.slice(i - 400, i + 300));
