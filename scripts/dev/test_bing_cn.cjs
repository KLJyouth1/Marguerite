// 复现 DDG/Bing 对中文科技新闻查询的真实返回（判定引擎质量根因）
const https = require('https');
const q = encodeURIComponent('今天的科技新闻 三条摘要');
// Bing 中国区
const req = https.get(`https://www.bing.com/search?q=${q}&mkt=zh-CN`, {
  headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36' }
}, res => {
  let d = '';
  res.on('data', c => d += c);
  res.on('end', () => {
    console.log('Bing status:', res.statusCode, '| html len:', d.length);
    // 提取 <li class="b_algo"> 结果数
    const algo = (d.match(/class="b_algo"/g) || []).length;
    console.log('b_algo 结果数:', algo);
    const titles = [...d.matchAll(/<h2><a[^>]*>(.*?)<\/a><\/h2>/g)].slice(0,3).map(m=>m[1].replace(/<[^>]+>/g,''));
    console.log('前3标题:', titles);
  });
});
req.on('error', e => console.log('ERR', e.message));
