// 打包脚本分析：package_app.cmd 如何组装 zip（pets 是否被复制）
const fs = require('fs');
const candidates = [
  'D:/Downloads/Marguerite/scripts/package_app.cmd',
  'D:/Downloads/Marguerite/scripts/package_app.cmd',
  'D:/Downloads/Marguerite/gui/scripts/package_app.cmd',
];
for (const c of candidates) {
  if (fs.existsSync(c)) {
    console.log('找到:', c);
    const src = fs.readFileSync(c, 'utf8');
    // 找 pets / webui / copy 相关行
    src.split('\n').forEach(line => {
      if (/pets|webui|copy|xcopy|Compress/i.test(line)) console.log('  ', line.trim().slice(0, 110));
    });
  }
}
