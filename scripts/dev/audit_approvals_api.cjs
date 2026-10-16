// 审计审批 HTTP API：GET /api/approvals
const http = require('http');
const fs = require('fs');
const token = fs.readFileSync(process.env.LOCALAPPDATA + '/Marguerite/gateway-token', 'utf8').trim();
const req = http.request({ hostname: '127.0.0.1', port: 18790, path: '/api/approvals', headers: { Authorization: 'Bearer ' + token } }, res => {
  let d = '';
  res.on('data', c => d += c);
  res.on('end', () => { console.log('status:', res.statusCode, 'body:', d.slice(0, 300)); });
});
req.on('error', e => console.log('ERR', e.message));
req.end();
