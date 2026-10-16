// zip 只有一个 exe → webui 嵌在 exe 内（Tauri embed）或运行时从别处加载
// 检查 exe 内嵌资源 + Tauri 协议如何服务 pets 图片
const fs = require('fs');
const conf = fs.readFileSync('D:/Downloads/Marguerite/gui/src-tauri/tauri.conf.json', 'utf8');
console.log(conf.slice(0, 1200));
