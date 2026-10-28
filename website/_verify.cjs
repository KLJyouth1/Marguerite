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
// 2026-10-27：主页改为以产品为首屏（下载器 / 安装器 / 工作台同一条叙事），
// 校验目标同步为新首屏标题，避免脚本与新文案脱节。
const SLOGAN = '让思考 成为你的工作台';

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

    // 懒加载的图在隐藏面板里不会触发加载：先滚到四入口并点开 TUI 标签，
    // 再测量，否则守卫测的是「未交互」这条错误路径（2026-10-27 实测踩过）。
    const entriesSection = page.locator('#entries');
    if (await entriesSection.count()) {
      await entriesSection.scrollIntoViewIfNeeded();
      const tuiTab = page.locator('#tab-tui');
      if (await tuiTab.count()) {
        await tuiTab.click();
        await page.waitForTimeout(500);
      }
      const cliTab = page.locator('#tab-cli');
      if (await cliTab.count()) { await cliTab.click(); await page.waitForTimeout(150); }
    }

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
        player: !!document.getElementById('playerStage'),
        acts: document.querySelectorAll('#playerStage .act').length,
        demo: !!document.querySelector('[data-demo-input]'),
        compare: !!document.querySelector('.cmp'),
        scene: !!document.getElementById('brandScene'),
        bpm: window.MARGUERITE_BEATS ? window.MARGUERITE_BEATS.beatBpm : null,
        notice: !!document.querySelector('[data-notice]'),
        ask: !!document.querySelector('[data-ask]'),
        faq: document.querySelectorAll('#faq details').length,
        dl: document.querySelectorAll('.dl__card').length,
        bar: !!document.querySelector('[data-scrollbar]'),
        dock: !!document.querySelector('[data-dock]'),
        // 2026-10-27 竞品借鉴轮新增结构：四入口 / 回环卡 / 信任区 / 门禁数字
        entryTabs: document.querySelectorAll('.entries__tab').length,
        entryPanels: document.querySelectorAll('.entries__panel').length,
        entryActive: document.querySelectorAll('.entries__panel.is-active').length,
        tuiImgLoaded: (() => {
          const img = document.querySelector('#entry-tui img');
          return !!img && img.complete && img.naturalWidth > 0;
        })(),
        loopback: !!document.querySelector('.loopback'),
        trustCards: document.querySelectorAll('.trust__card').length,
        counters: document.querySelectorAll('[data-count]').length,
        gateLines: document.querySelectorAll('.term--gate .term__line').length,
        navLinks: document.querySelectorAll('.nav__links a').length,
        bento: document.querySelectorAll('#capabilities .cards .card').length,
        honestCards: document.querySelectorAll('.cards--honest .card').length,
        // 2026-10-28 案例复盘区：六个可折叠任务卡，默认只展开第一个
        caseCards: document.querySelectorAll('.cf__item').length,
        caseOpen: document.querySelectorAll('.cf__item[open]').length,
        // 2026-10-28 开发史：四章时间轴 + 24 小时柱（缺一根就会出现「午夜没有刻度」）
        storyChaps: document.querySelectorAll('#story .chap').length,
        storyBars: document.querySelectorAll('#story .curve__bars > span').length,
        storyLedger: document.querySelectorAll('#story .ledger__i').length,
        // 右侧夜景插画：内联 SVG，不能是空的（空 svg 会让右栏看着像没画完）
        storyScene: (() => {
          const svg = document.querySelector('#story .story__scene svg');
          return !!svg && svg.getBoundingClientRect().height > 120;
        })(),
        // bento 尾行不得留空缺：6 列网格下每张卡的跨列总和必须是 6 的整数倍
        // ⚠️ `grid-column: span N` 简写的值落在 gridColumnStart 上，gridColumnEnd 是 "auto"
        //    （2026-10-27 第一版读了 End，把每张卡当 span 1，制造出假红）
        bentoRowsFull: (() => {
          const cards = Array.from(document.querySelectorAll('#capabilities .cards .card'));
          if (!cards.length) return true;
          const grid = getComputedStyle(document.querySelector('#capabilities .cards'));
          const cols = (grid.gridTemplateColumns || '').split(/\s+/).filter(Boolean).length;
          if (!cols || cols === 1) return true; // 窄屏单列：无从留空缺
          const spanOf = (card) => {
            const cs = getComputedStyle(card);
            const hit = /span\s+(\d+)/.exec(cs.gridColumnStart + ' ' + cs.gridColumnEnd);
            const span = hit ? parseInt(hit[1], 10) : 1;
            return span > 0 ? span : 1;
          };
          let used = 0;
          const full = cards.every((card) => {
            const span = spanOf(card);
            if (used + span > cols) used = span; else used += span; // 放不下则换行
            if (used === cols) { used = 0; }
            return used <= cols;
          });
          return full && used === 0; // 收尾必须正好填满，否则末行有空缺
        })(),
        // 2026-10-27 修：整页 scrollWidth 守卫看不见这两类缺陷（页面不溢出 ≠ 文字可读）
        // ① 中文 min-content 宽度 = 1 字符，导航链接作为可收缩 flex item 会被压成逐字换行
        navWrapped: Array.from(document.querySelectorAll('.nav__links a')).filter((a) => {
          const r = a.getBoundingClientRect();
          if (!r.width) return false; // 折叠态不参与
          const lh = parseFloat(getComputedStyle(a).lineHeight) || 21;
          return r.height > lh * 1.5;
        }).length,
        // ② 终端/JSON 块自带 overflow-x，长行会被静默裁在半截而不触发整页溢出
        termClipped: Array.from(document.querySelectorAll('.term'))
          .filter((t) => t.scrollWidth > t.clientWidth + 1).length,
      };
    });

    const structureOk = m.player && m.acts === 7 && m.demo && m.compare && m.scene && m.bpm === 56 &&
      m.notice && m.ask && m.faq === 7 && m.dl === 3 && m.bar && m.dock &&
      m.entryTabs === 4 && m.entryPanels === 4 && m.entryActive === 1 && m.tuiImgLoaded &&
      m.loopback && m.trustCards === 3 && m.counters === 8 && m.gateLines === 7 &&
      m.navLinks === 12 && m.bento === 8 && m.honestCards === 4 && m.bentoRowsFull &&
      m.caseCards === 6 && m.caseOpen === 1 &&
      m.storyChaps === 4 && m.storyBars === 24 && m.storyLedger === 6 && m.storyScene;
    if (!structureOk) {
      allPass = false;
      errors.push(s.name + ' structure: ' + JSON.stringify(m));
    }

    const sloganOk = m.text === SLOGAN;
    const noOverflow = !m.overflowX;
    const typographyOk = m.navWrapped === 0 && m.termClipped === 0;
    if (!sloganOk) { allPass = false; errors.push(s.name + ' slogan mismatch: ' + JSON.stringify(m.text)); }
    if (!typographyOk) {
      allPass = false;
      errors.push(s.name + ' typography: 导航逐字换行 ' + m.navWrapped + ' 项 / 终端块裁切 ' + m.termClipped + ' 处');
    }
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
    console.log('  排版可读    : ' + (typographyOk ? 'PASS（导航单行 · 终端块不裁切）'
      : 'FAIL（导航逐字换行 ' + m.navWrapped + ' 项 / 终端块裁切 ' + m.termClipped + ' 处）'));
    console.log('  事实条目    : ' + m.facts + (m.facts === 4 ? ' OK' : ' *** 期望 4 ***'));
    console.log('  结构（短片/演示/对比/公告/提问/FAQ/下载）: ' + (structureOk ? 'PASS（7 幕 · 56 BPM · FAQ 7 · 下载 3）' : 'FAIL ' + JSON.stringify(m)));

    try {
      await page.screenshot({ path: '_shot-' + s.name + '.png', fullPage: false, animations: 'disabled', timeout: 4000 });
    } catch (e) {
      console.log('  [screenshot skipped: ' + (e && e.message ? e.message.split('\n')[0] : 'env hang') + ']');
    }
    await ctx.close();
  }

  // 1024–1180 是历史缺陷所在的窄带：桌面导航刚折叠、内容仍双栏、终端块最窄。
  // 这三档不截图（截图交付物固定为三张），只跑排版与溢出断言。
  for (const w of [1024, 1100, 1180]) {
    const bandCtx = await browser.newContext({ viewport: { width: w, height: 900 } });
    const bandPage = await bandCtx.newPage();
    await bandPage.goto(URL, { waitUntil: 'load' });
    await bandPage.waitForTimeout(300);
    const band = await bandPage.evaluate(() => {
      const navWrapped = Array.from(document.querySelectorAll('.nav__links a')).filter((a) => {
        const r = a.getBoundingClientRect();
        if (!r.width) return false;
        const lh = parseFloat(getComputedStyle(a).lineHeight) || 21;
        return r.height > lh * 1.5;
      }).length;
      return {
        navWrapped,
        termClipped: Array.from(document.querySelectorAll('.term'))
          .filter((t) => t.scrollWidth > t.clientWidth + 1).length,
        overflowX: document.documentElement.scrollWidth > window.innerWidth,
      };
    });
    const bandOk = band.navWrapped === 0 && band.termClipped === 0 && !band.overflowX;
    if (!bandOk) { allPass = false; errors.push(w + 'px 窄带: ' + JSON.stringify(band)); }
    console.log('--- 窄带巡检 ' + w + 'px（不截图）---  ' + (bandOk ? 'PASS' : 'FAIL ' + JSON.stringify(band)));
    await bandCtx.close();
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
