// 实测：向运行中的网关请求 pets 图片，看返回 200 还是 404/HTML fallback
const http = require('http');
const req = http.get('http://127.0.0.1:18790/control/pets/skin-blue.jpg', res => {
  console.log('status:', res.statusCode, '| content-type:', res.headers['content-type']);
  let size = 0;
  res.on('data', c => size += c.length);
  res.on('end', () => {
    console.log('body bytes:', size);
    if (res.headers['content-type']?.includes('html')) console.log('← 返回的是 HTML（SPA fallback），图片未命中');
  });
});
req.on('error', e => console.log('ERR:', e.message));
