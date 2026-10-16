// 检查 webui/dist（源）是否包含 pets —— Tauri resources 打包的是这个目录
const fs = require('fs');
const d = 'D:/Downloads/Marguerite/gui/src-tauri/webui/dist';
if (fs.existsSync(d)) {
  console.log('gui/src-tauri/webui/dist 存在');
  const pets = d + '/pets';
  console.log('pets:', fs.existsSync(pets) ? fs.readdirSync(pets).join(', ') : '不存在 ← 打包缺图根因');
  // 对比主 dist
  const m = 'D:/Downloads/Marguerite/webui/dist';
  console.log('--- 主仓 webui/dist/pets:', fs.existsSync(m + '/pets') ? fs.readdirSync(m + '/pets').join(', ') : '不存在');
} else {
  console.log('gui/src-tauri/webui/dist 不存在（构建时从主仓复制?）');
}
