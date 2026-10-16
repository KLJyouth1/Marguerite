// 检查打包产物（Tauri dist）里的 pets 资源 —— 用户「打包后桌宠还是图片」的根因
const fs = require('fs');
const candidates = [
  'D:/Downloads/Marguerite/dist',
  'D:/Downloads/Marguerite/gui/src-tauri/dist',
  'D:/Downloads/Marguerite/marguerite/target/release',
];
for (const c of candidates) {
  if (fs.existsSync(c)) {
    console.log('存在:', c);
    const sub = c + '/pets';
    if (fs.existsSync(sub)) console.log('  pets:', fs.readdirSync(sub).slice(0,6).join(','));
    else console.log('  pets/ 不存在');
  }
}
