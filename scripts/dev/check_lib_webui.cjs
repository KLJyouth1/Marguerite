// 关键发现：Tauri 窗口直接加载 http://localhost:18790/control/（网关服务），
// 不用 tauri 协议 → 图片来自网关静态目录。检查打包版网关的 webui 目录解析逻辑
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/gui/src-tauri/src/lib.rs', 'utf8');
const hits = [...src.matchAll(/webui[^\n]*/g)].slice(0, 10);
hits.forEach(h => console.log(h[0].trim().slice(0, 120)));
