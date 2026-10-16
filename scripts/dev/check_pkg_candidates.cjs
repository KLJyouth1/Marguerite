// 检查打包版（gui exe 旁）的 webui/dist 是否含 pets —— 打包后 404 的根因
const fs = require('fs');
const path = require('path');
// 模拟 resolve_webui_dir 的候选顺序（exe = zip 解压目录）
const exeDir = 'D:/Downloads/Marguerite/dist';
const candidates = [];
// resource_dir 对 zip 绿色包通常 = exe 目录
candidates.push(path.join(exeDir, 'webui/dist'));
let base = exeDir;
for (let i = 0; i < 6; i++) {
  base = path.dirname(base);
  candidates.push(path.join(base, 'webui/dist'));
}
for (const c of candidates) {
  const hasIndex = fs.existsSync(c + '/index.html');
  const hasPets = fs.existsSync(c + '/pets');
  console.log((hasIndex ? '✓' : '✗'), c, '| pets:', hasPets ? '有' : '无');
}
