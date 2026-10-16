// 关键发现：DDG (duckduckgo.com) 从当前网络直连超时（被墙）
// → run_web_search 主引擎 DDG 恒超时 15s → 回退 Bing → Bing 302 重定向（反爬）
// → 最终返回的是搜狗/聚合站等低质结果（web_research 的 readability 深读抓到什么算什么）

// 验证：国内可直连的搜索引擎质量测试
const https = require('https');

function test(name, url, headers, checkFn) {
  return new Promise(resolve => {
    const req = https.get(url, { headers, timeout: 10000 }, res => {
      let d = '';
      res.on('data', c => d += c);
      res.on('end', () => {
        const r = checkFn(res.statusCode, d);
        console.log(`${name}: status=${res.statusCode} ${r}`);
        resolve();
      });
    });
    req.on('timeout', () => { console.log(`${name}: TIMEOUT`); req.destroy(); resolve(); });
    req.on('error', e => { console.log(`${name}: ERR ${e.message.slice(0,50)}`); resolve(); });
  });
}

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36';
(async () => {
  await test('Bing CN', 'https://cn.bing.com/search?q=%E4%BB%8A%E5%A4%A9%E7%9A%84%E7%A7%91%E6%8A%80%E6%96%B0%E9%97%BB&mkt=zh-CN', { 'User-Agent': UA },
    (s, d) => `len=${d.length} b_algo=${(d.match(/class="b_algo"/g)||[]).length}`);
  await test('百度', 'https://www.baidu.com/s?wd=%E4%BB%8A%E5%A4%A9%E7%9A%84%E7%A7%91%E6%8A%80%E6%96%B0%E9%97%BB', { 'User-Agent': UA },
    (s, d) => `len=${d.length} result=${(d.match(/class="result/g)||[]).length}`);
})();
