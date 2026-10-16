// 模拟拆解步骤请求（带 tools + 大 max_tokens），定位 120s 挂起
const https = require('https');
const fs = require('fs');
const key = fs.readFileSync('D:/Downloads/Marguerite/marguerite/.env', 'utf8').match(/MARGUERITE_API_KEY=(.+)/)[1].trim();
const body = JSON.stringify({
  model: 'deepseek-v4-flash',
  messages: [
    { role: 'system', content: '你是计划执行子代理。'.repeat(50) },
    { role: 'user', content: '【计划子步骤 step-1｜设计系统架构】整体目标：个人博客系统（文章/评论/标签）。请完成该子步骤。' }
  ],
  max_tokens: 393216,
  tools: [{ type: 'function', function: { name: 'web_search', description: '搜索', parameters: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] } } }],
  stream: false
});
const t0 = Date.now();
const req = https.request({
  hostname: 'api.deepseek.com', path: '/chat/completions', method: 'POST',
  headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer ' + key, 'Content-Length': Buffer.byteLength(body) },
  timeout: 150000
}, res => {
  let d = '';
  res.on('data', c => d += c);
  res.on('end', () => {
    console.log('耗时:', Date.now() - t0, 'ms status:', res.statusCode);
    console.log(d.slice(0, 250));
    process.exit(0);
  });
});
req.on('timeout', () => { console.log('REQUEST-TIMEOUT at', Date.now() - t0, 'ms'); req.destroy(); process.exit(2); });
req.on('error', e => { console.log('REQ ERR:', e.message); process.exit(3); });
req.write(body); req.end();
