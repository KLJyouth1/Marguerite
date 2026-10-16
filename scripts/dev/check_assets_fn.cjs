// 检查 control_assets 静态服务是否允许 .jpg/.png（pets 图片 404 的另一可能根因）
const fs = require('fs');
const src = fs.readFileSync('D:/Downloads/Marguerite/core/src/gateway/server.rs', 'utf8');
const i = src.indexOf('async fn control_assets');
console.log(src.slice(i, i + 1800));
