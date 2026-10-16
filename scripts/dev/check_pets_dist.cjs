// 检查打包产物里 pet 图片是否存在（多 webui 打包问题的核心）
const fs = require('fs');
// 1. webui/dist（源站）
const d1 = 'D:/Downloads/Marguerite/webui/dist/pets';
console.log('webui/dist/pets:', fs.existsSync(d1) ? fs.readdirSync(d1).join(', ') : '不存在');
// 2. gateway 静态目录（网关实际服务的目录）
const gw = process.env.LOCALAPPDATA + '/Marguerite/webui';
console.log('gateway webui 目录存在:', fs.existsSync(gw));
if (fs.existsSync(gw)) {
  const entries = fs.readdirSync(gw).slice(0, 10);
  console.log('内容:', entries.join(', '));
  const pets = gw + '/pets';
  console.log('gateway pets:', fs.existsSync(pets) ? fs.readdirSync(pets).join(', ') : 'pets/ 不存在 ← 图片 404 根因');
}
