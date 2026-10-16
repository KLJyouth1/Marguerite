// DDG 中文查询质量测试（POST 修正版）
const https = require('https');
const q = '今天的科技新闻';
const body = 'q=' + encodeURIComponent(q);
const req = https.request('https://html.duckduckgo.com/html/', {
  method: 'POST',
  headers: {
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36',
    'Content-Type': 'application/x-www-form-urlencoded',
    'Content-Length': Buffer.byteLength(body)
  }
}, res => {
  let d = '';
  res.on('data', c => d += c);
  res.on('end', () => {
    console.log('DDG status:', res.statusCode, '| len:', d.length);
    const results = (d.match(/class="result"/g) || []).length;
    console.log('result 块数:', results);
    console.log('含 anomaly/captcha:', /anomaly|captcha|challenge/i.test(d));
    const snippets = [...d.matchAll(/class="result__snippet"[^>]*>([\s\S]*?)<\/a>/g)].slice(0,2).map(m=>m[1].replace(/<[^>]+>/g,'').replace(/\s+/g,' ').slice(0,80));
    console.log('前2摘要:', snippets);
  });
});
req.write(body);
req.end();
