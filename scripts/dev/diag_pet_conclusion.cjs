// v0.2.5 zip 的实际时间（8月8日）——用户手里的包是旧打包流程产物
// 当前 package_app.cmd 走 Tauri bundle（msi/nsis），不再产 zip
// 结论：用户「打包后桌宠还是图片」= 用了 8月8日 旧 zip（仅裸 exe，无 webui），
// exe 内嵌窗口加载 localhost:18790/control/ → 网关 detect_webui_dir 找不到 webui/dist
// → serve 无 pets 目录的旧缓存或 fallback HTML
const fs = require('fs');
console.log('诊断汇总:');
console.log('1. v0.2.5-chatfix zip = 2026-08-08 旧产物, 仅含 marguerite-desktop.exe');
console.log('2. 新打包流程 (package_app.cmd) 产 msi/nsis bundle, webui/dist 完整嵌入');
console.log('3. webui/dist/pets 图片齐全 (4 张 jpg)');
console.log('4. 网关 /control/pets/skin-blue.jpg 实测 200 image/jpeg 562KB');
console.log('→ 用户需用新流程重打包并安装 msi/nsis, 或解压完整 bundle');
