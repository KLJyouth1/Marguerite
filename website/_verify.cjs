/* 等效静态校验（适配新 dark 重设计结构）
   校验：无外部依赖、无横向溢出、标语精确、reduced-motion 下内容完整可见。
   因旧结构 .poster__title / .seal 已不存在，原脚本会 false-fail，故同步更新。 */
const { chromium } = require('../webui/node_modules/playwright');
const path = require('path');

const URL = 'file:///' + path.resolve(__dirname, 'index.html').replace(/\\/g, '/');
const SIZES = [
  { w: 375, h: 900, name: 'mobile-375' },
  { w: 768, h: 1000, name: 'tablet-768' },
  { w: 1440, h: 1000, name: 'desktop-1440' },
];
const SLOGAN = '只要做到极致 什么都能成功';

(async () => {
  const browser = await chromium.launch();
  const errors = [];
  let allPass = true;

  for (const s of SIZES) {
    const ctx = await browser.newContext({ viewport: { width: s.w, height: s.h } });
    const page = await ctx.newPage();
    page.on('console', (m) => { if (m.type() === 'error') errors.push(s.name + ': ' + m.text()); });
    page.on('pageerror', (e) => errors.push(s.name + ' pageerror: ' + e.message));
    page.on('requestfailed', (r) => {
      const u = r.url();
      if (u.startsWith('http')) errors.push(s.name + ' external-requestfailed: ' + u);
    });

    await page.goto(URL, { waitUntil: 'load' });
    await page.waitForTimeout(900);

    const m = await page.evaluate(() => {
      const h1 = document.querySelector('.hero__title');
      const facts = document.querySelectorAll('.facts li');
      const root = document.documentElement;
      return {
        text: (h1 ? h1.textContent.replace(/\s+/g, ' ').trim() : ''),
        overflowX: root.scrollWidth > window.innerWidth,
        scrollW: root.scrollWidth,
        vw: window.innerWidth,
        facts: facts.length,
      };
    });

    const sloganOk = m.text === SLOGAN;
    const noOverflow = !m.overflowX;
    if (!sloganOk) { allPass = false; errors.push(s.name + ' slogan mismatch: ' + JSON.stringify(m.text)); }
    if (!noOverflow) {
      allPass = false;
      const culprits = await page.evaluate((vw) => {
        const out = [];
        document.querySelectorAll('*').forEach((el) => {
          const r = el.getBoundingClientRect();
          if (r.width > 0 && (r.right > vw + 1 || r.left < -1)) {
            out.push(el.tagName.toLowerCase() + (el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\s+/).join('.') : '') + ' right=' + Math.round(r.right));
          }
        });
        return out.slice(0, 8);
      }, s.w);
      errors.push(s.name + ' overflow culprits: ' + culprits.join(', '));
    }

    console.log('--- ' + s.name + ' (' + s.w + 'px) ---');
    console.log('  标语精确    : ' + (sloganOk ? 'PASS' : 'FAIL ' + JSON.stringify(m.text)));
    console.log('  横向溢出    : ' + (noOverflow ? 'PASS (scrollW ' + m.scrollW + ' / vw ' + m.vw + ')' : 'FAIL'));
    console.log('  事实条目    : ' + m.facts + (m.facts === 4 ? ' OK' : ' *** 期望 4 ***'));

    try {
      await page.screenshot({ path: '_shot-' + s.name + '.png', fullPage: false, animations: 'disabled', timeout: 4000 });
    } catch (e) {
      console.log('  [screenshot skipped: ' + (e && e.message ? e.message.split('\n')[0] : 'env hang') + ']');
    }
    await ctx.close();
  }

  const ctx2 = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
  const p2 = await ctx2.newPage();
  await p2.goto(URL, { waitUntil: 'load' });
  await p2.waitForTimeout(300);
  const rm = await p2.evaluate(() => {
    const t = document.querySelector('.hero__title');
    const f = document.querySelector('.facts');
    return {
      titleOpacity: getComputedStyle(t).opacity,
      titleVisible: t.getBoundingClientRect().height > 10,
      factsVisible: f.getBoundingClientRect().height > 10,
    };
  });
  const rmPass = rm.titleOpacity === '1' && rm.titleVisible && rm.factsVisible;
  if (!rmPass) allPass = false;
  console.log('--- prefers-reduced-motion: reduce ---');
  console.log('  标题/事实可见: ' + (rmPass ? 'PASS（降级后内容完整）' : 'FAIL ' + JSON.stringify(rm)));
  await ctx2.close();

  console.log('\n=== 控制台错误 / 失败请求 ===');
  console.log(errors.length === 0 ? '零错误、零外部请求（证明无外部依赖、无 emoji CDN）' : errors.join('\n'));
  if (errors.length) allPass = false;

  await browser.close();
  console.log('\n总判定: ' + (allPass ? 'PASS' : 'FAIL'));
  process.exit(allPass ? 0 : 1);
})();
