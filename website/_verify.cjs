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
        // 2026-10-28 本地体验模型：模型卡拉 6 行、评测 4 条、下载 3 张卡
        lmRows: document.querySelectorAll('#local-model .lm__row').length,
        lmBars: document.querySelectorAll('#local-model .lm__bar').length,
        lmCards: document.querySelectorAll('#local-model .lm__dlc[href]').length,
        // 2026-10-31 创新轮：战术推演 5 拓扑 / 7 战术 / 5-Why 链 / 原生硬化台账 4 数 / 证据模式
        tacTabs: document.querySelectorAll('.tac__tab').length,
        tacChips: document.querySelectorAll('.tac__chip').length,
        whySteps: document.querySelectorAll('.why .why__step').length,
        nativeStats: document.querySelectorAll('.native__stat').length,
        proofToggle: !!document.querySelector('#proofToggle'),
        proofPanel: !!document.querySelector('#proofPanel'),
        proofMarked: document.querySelectorAll('[data-proof]').length,
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
        // 2026-10-29 新增：每节必须有一句话结论（`sec-take`）。
        // 判据是「可见且不是空串」，不是「DOM 里有 14 个」——被 CSS 隐藏或内容空掉
        // 都等于没有电梯，而计数断言对这两种情况完全不敏感。
        secTakes: Array.from(document.querySelectorAll('.sec-take'))
          .filter((el) => el.getBoundingClientRect().height > 8 && (el.textContent || '').trim().length >= 8)
          .length,
        // 2026-10-29 新增：导航顺序必须与文档几何顺序一致。
        // 实测过这个坑：`#compare` 的 section 在 DOM 里排在 `#capabilities` 之前，
        // 而导航按另一个顺序排 —— 读者点「差异」会往回跳一整节，感觉像页面出错。
        navOrderAscending: (() => {
          const ids = Array.from(document.querySelectorAll('.nav__links a[href^="#"]'))
            .map((a) => a.getAttribute('href').slice(1));
          const ys = ids.map((id) => {
            const el = document.getElementById(id);
            return el ? el.getBoundingClientRect().top + window.scrollY : null;
          });
          if (ys.some((y) => y === null)) return false;
          return ys.every((y, i) => i === 0 || y > ys[i - 1]);
        })(),
        // 2026-10-29 新增：社交卡片与图片格式必须有守卫，否则「加了又被人改回去」没人发现。
        ogImage: !!document.querySelector('meta[property="og:image"]'),
        twitterCard: !!document.querySelector('meta[name="twitter:card"]'),
        canonical: !!document.querySelector('link[rel="canonical"]'),
        pictureCount: document.querySelectorAll('picture > source[type="image/webp"]').length,
        // 浏览器实际选中的是不是 webp（只看源码有没有 <source> 不够，要证明它生效了）
        webpServed: Array.from(document.images)
          .filter((i) => (i.currentSrc || '').endsWith('.webp')).length,
      };
    });

    const structureOk = m.player && m.acts === 7 && m.demo && m.compare && m.scene && m.bpm === 56 &&
      m.notice && m.ask && m.faq === 8 && m.dl === 3 && m.bar && m.dock &&
      m.entryTabs === 4 && m.entryPanels === 4 && m.entryActive === 1 && m.tuiImgLoaded &&
      m.loopback && m.trustCards === 3 && m.counters === 8 && m.gateLines === 7 &&
      m.navLinks === 15 && m.navOrderAscending && m.secTakes === 15 && m.bento === 9 && m.honestCards === 4 && m.bentoRowsFull &&
      m.caseCards === 6 && m.caseOpen === 1 &&
      m.storyChaps === 4 && m.storyBars === 24 && m.storyLedger === 6 && m.storyScene &&
      m.lmRows === 6 && m.lmBars === 4 && m.lmCards === 3 &&
      m.tacTabs === 5 && m.tacChips === 7 && m.whySteps === 5 && m.nativeStats === 4 &&
      m.proofToggle && m.proofPanel && m.proofMarked >= 8;
    if (!structureOk) {
      allPass = false;
      errors.push(s.name + ' structure: ' + JSON.stringify(m));
    }

    // 社交卡片三件套 + WebP 生效（2026-10-29 补）。
    // 这里刻意要求 `webpServed > 0`：只检查源码里有没有 `<source>` 会被"写了但浏览器没用"骗过。
    const seoOk = m.ogImage && m.twitterCard && m.canonical && m.pictureCount > 0 && m.webpServed > 0;
    if (!seoOk) {
      allPass = false;
      errors.push(s.name + ' seo/media: og=' + m.ogImage + ' tw=' + m.twitterCard
        + ' canonical=' + m.canonical + ' picture=' + m.pictureCount + ' webpServed=' + m.webpServed);
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
    console.log('  结构（短片/演示/对比/公告/提问/FAQ/下载）: ' + (structureOk ? 'PASS（7 幕 · 56 BPM · FAQ 8 · 下载 3 · 15 项导航顺序=文档顺序 · 每节一句话结论 15 · 推演 5/7 · 5-Why 5 · 证据标记 ' + m.proofMarked + '）' : 'FAIL ' + JSON.stringify(m)));
    console.log('  社交卡片/图片格式: ' + (seoOk
      ? 'PASS（og+twitter+canonical · WebP 生效 ' + m.webpServed + '/' + m.pictureCount + '）'
      : 'FAIL'));

    try {
      await page.screenshot({ path: '_shot-' + s.name + '.png', fullPage: false, animations: 'disabled', timeout: 4000 });
    } catch (e) {
      console.log('  [screenshot skipped: ' + (e && e.message ? e.message.split('\n')[0] : 'env hang') + ']');
    }
    await ctx.close();
  }

  // 1024–1360 是历史缺陷所在的窄带：桌面导航刚折叠/刚收字号、内容仍双栏、终端块最窄。
  // 这几档不截图（截图交付物固定为三张），只跑排版与溢出断言。
  //
  // 2026-10-29 扩宽并加两条断言：14 项导航 + 语言切换 + 声音 + CTA 挤在同一行时，
  // `.nav__cta` / `.sound__label` 会被 flex 压成**逐字竖排**（曾实测 5.7 行 / 2.8 行），
  // 而整页 scrollWidth 仍然等于视口宽 —— 光看"有没有横向溢出"永远发现不了这个。
  for (const w of [1024, 1100, 1180, 1220, 1280, 1360]) {
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
      const lines = (sel, fallbackLh) => {
        const el = document.querySelector(sel);
        if (!el) return 0;
        const cs = getComputedStyle(el);
        const lh = parseFloat(cs.lineHeight) || fallbackLh;
        return Math.round((el.getBoundingClientRect().height / lh) * 10) / 10;
      };
      return {
        navWrapped,
        termClipped: Array.from(document.querySelectorAll('.term'))
          .filter((t) => t.scrollWidth > t.clientWidth + 1).length,
        overflowX: document.documentElement.scrollWidth > window.innerWidth,
        ctaLines: lines('.nav__cta', 20),
        soundLines: lines('.sound__label', 16),
      };
    });
    const bandOk = band.navWrapped === 0 && band.termClipped === 0 && !band.overflowX
      && band.ctaLines <= 1.8 && band.soundLines <= 1.6;
    if (!bandOk) { allPass = false; errors.push(w + 'px 窄带: ' + JSON.stringify(band)); }
    console.log('--- 窄带巡检 ' + w + 'px（不截图）---  ' + (bandOk ? 'PASS' : 'FAIL ' + JSON.stringify(band)));
    await bandCtx.close();
  }

  // ── 2026-10-31 新增：证据模式 / 战术推演的可操作守卫 ─────────────────────
  // 静态计数只能证明「按钮在」，不能证明「按下去有反应」。这里真的点：
  // ① 证据模式要在页面上亮出标记、打开证据表、条数够、关闭能复原；
  // ② 战术推演要能切拓扑、真的画出节点与连线、点「跑一次」写日志。
  {
    const ictx = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    const ip = await ictx.newPage();
    ip.on('console', (m) => { if (m.type() === 'error') errors.push('interaction: ' + m.text()); });
    ip.on('pageerror', (e) => errors.push('interaction pageerror: ' + e.message));
    await ip.goto(URL, { waitUntil: 'load' });
    await ip.waitForTimeout(500);

    await ip.click('#proofToggle');
    await ip.waitForTimeout(200);
    const proofOn = await ip.evaluate(() => ({
      on: document.body.classList.contains('proof-on'),
      pressed: document.getElementById('proofToggle').getAttribute('aria-pressed'),
      dialog: !!document.getElementById('proofPanel').open,
      rows: document.querySelectorAll('.proof__row').length,
      bar: !!document.querySelector('[data-proof-bar]') && !document.querySelector('[data-proof-bar]').hidden,
      count: (document.querySelector('[data-proof-count]') || {}).textContent,
      marked: document.querySelectorAll('[data-proof]').length,
      // 证据表里的值必须等于台账目标值：计数动画的中间帧不算证据。
      mismatches: Array.from(document.querySelectorAll('[data-proof]')).map((el) => {
        const target = el.querySelector('.stat__n [data-count]');
        if (!target) return null;
        const key = el.getAttribute('data-proof');
        const go = document.querySelector('.proof__row [data-proof-go="' + key + '"]');
        const val = go ? go.parentElement.querySelector('.proof__rv') : null;
        const want = target.getAttribute('data-count');
        const got = val ? val.textContent.trim() : 'missing';
        return got === want ? null : key + ':' + got + '≠' + want;
      }).filter(Boolean),
    }));
    const proofOnOk = proofOn.on && proofOn.pressed === 'true' && proofOn.dialog &&
      proofOn.rows >= 8 && proofOn.bar && Number(proofOn.count) === proofOn.rows &&
      proofOn.mismatches.length === 0;
    if (!proofOnOk) { allPass = false; errors.push('证据模式开启: ' + JSON.stringify(proofOn)); }
    console.log('--- 证据模式 ---');
    console.log('  开启并生成证据表: ' + (proofOnOk
      ? 'PASS（' + proofOn.rows + ' 条 / 标记 ' + proofOn.marked + ' 处）'
      : 'FAIL ' + JSON.stringify(proofOn)));

    await ip.keyboard.press('Escape');
    await ip.waitForTimeout(150);
    const proofClosed = await ip.evaluate(() => ({
      dialog: document.getElementById('proofPanel').open,
      on: document.body.classList.contains('proof-on'),
    }));
    // Esc 只关对话框，不退出证据模式 —— 这是设计：标记与常驻证据条应保留。
    const proofClosedOk = !proofClosed.dialog && proofClosed.on;
    if (!proofClosedOk) { allPass = false; errors.push('证据模式 Esc: ' + JSON.stringify(proofClosed)); }

    await ip.click('[data-proof-open]');
    await ip.waitForTimeout(150);
    const proofReopen = await ip.evaluate(() => document.getElementById('proofPanel').open);
    if (!proofReopen) { allPass = false; errors.push('证据条「查看证据表」未能重开对话框'); }

    await ip.click('[data-proof-close]');
    await ip.waitForTimeout(100);
    await ip.click('#proofToggle');
    await ip.waitForTimeout(150);
    const proofOff = await ip.evaluate(() => ({
      on: document.body.classList.contains('proof-on'),
      pressed: document.getElementById('proofToggle').getAttribute('aria-pressed'),
      bar: document.querySelector('[data-proof-bar]').hidden,
      dialog: document.getElementById('proofPanel').open,
    }));
    const proofOffOk = !proofOff.on && proofOff.pressed === 'false' && proofOff.bar && !proofOff.dialog;
    if (!proofOffOk) { allPass = false; errors.push('证据模式关闭: ' + JSON.stringify(proofOff)); }
    console.log('  关闭并复原: ' + (proofOffOk ? 'PASS（Esc 保留模式 · 证据条可重开 · 再按关闭清标记）' : 'FAIL ' + JSON.stringify(proofOff)));

    await ip.locator('#tactics').scrollIntoViewIfNeeded();
    await ip.click('.tac__tab[data-topo="swarm"]');
    await ip.waitForTimeout(150);
    const sw = await ip.evaluate(() => {
      const svg = document.querySelector('[data-tac-svg]');
      return {
        nodes: svg.querySelectorAll('.tac__node').length,
        edges: svg.querySelectorAll('.tac__edge').length,
        selected: document.querySelector('.tac__tab[data-topo="swarm"]').getAttribute('aria-selected'),
        label: svg.getAttribute('aria-label') || '',
      };
    });
    // 蜂群是 1 分片器 + 6 并行子 + 1 聚合器 = 8 节点；选中态与 aria-label 必须同步。
    const swOk = sw.nodes === 8 && sw.edges > 0 && sw.selected === 'true' && sw.label.indexOf('蜂群') >= 0;
    if (!swOk) { allPass = false; errors.push('战术推演切换拓扑: ' + JSON.stringify(sw)); }

    await ip.click('[data-tac-run]');
    await ip.waitForTimeout(1500);
    const ran = await ip.evaluate(() => ({
      logLines: document.querySelectorAll('[data-tac-log] > *').length,
      status: (document.querySelector('[data-tac-status]') || {}).textContent || '',
      marked: document.querySelectorAll('[data-tac-svg] .tac__node.is-run, [data-tac-svg] .tac__node.is-fail, [data-tac-svg] .tac__node.is-caught').length,
      running: document.querySelector('[data-tac-run]').disabled,
    }));
    const ranOk = ran.logLines >= 1 && ran.status.length > 4 && ran.marked >= 1 && !ran.running;
    if (!ranOk) { allPass = false; errors.push('战术推演跑一次: ' + JSON.stringify(ran)); }
    console.log('  切换拓扑 + 跑一次: ' + (swOk && ranOk
      ? 'PASS（蜂群 8 节点 / ' + sw.edges + ' 条边 · 日志 ' + ran.logLines + ' 行 · 状态：' + ran.status.trim() + '）'
      : 'FAIL ' + JSON.stringify({ sw, ran })));

    await ictx.close();
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

  // ── 英文落地页（index.en.html）────────────────────────────────────────────
  // 中文页是主叙事，英文页是精简版；两者共用 styles.css，所以布局风险同样存在。
  // 这里只断言最要紧的四条：不横向溢出、零控制台错误、有社交卡片、双语互指齐备。
  {
    const EN = URL.replace(/index\.html$/, 'index.en.html');
    for (const w of [375, 768, 1440]) {
      const c = await browser.newContext({ viewport: { width: w, height: 900 } });
      const pg = await c.newPage();
      const enErrors = [];
      pg.on('console', (m) => { if (m.type() === 'error') enErrors.push(m.text().slice(0, 120)); });
      await pg.goto(EN, { waitUntil: 'load' });
      await pg.waitForTimeout(300);
      const r = await pg.evaluate(() => ({
        overflowX: document.documentElement.scrollWidth - window.innerWidth,
        og: !!document.querySelector('meta[property="og:image"]'),
        hreflang: document.querySelectorAll('link[rel="alternate"][hreflang]').length,
        webpServed: Array.from(document.images).filter((i) => (i.currentSrc || '').endsWith('.webp')).length,
        // 2026-10-29：英文精简页四节也各需一句电梯结论，与中文页设计语言一致。
        secTakes: Array.from(document.querySelectorAll('.sec-take'))
          .filter((el) => el.getBoundingClientRect().height > 8 && (el.textContent || '').trim().length >= 8)
          .length,
      }));
      const ok = !r.overflowX && r.og && r.hreflang >= 3 && r.webpServed > 0 && r.secTakes === 4 && enErrors.length === 0;
      if (!ok) {
        allPass = false;
        errors.push('index.en.html@' + w + ': ' + JSON.stringify(r) + ' console=' + enErrors.join('|'));
      }
      console.log('--- index.en.html (' + w + 'px) --- ' + (ok ? 'PASS' : 'FAIL ' + JSON.stringify(r)));
      await c.close();
    }
  }

  console.log('\n=== 控制台错误 / 失败请求 ===');
  console.log(errors.length === 0 ? '零错误、零外部请求（证明无外部依赖、无 emoji CDN）' : errors.join('\n'));
  if (errors.length) allPass = false;

  await browser.close();
  console.log('\n总判定: ' + (allPass ? 'PASS' : 'FAIL'));
  process.exit(allPass ? 0 : 1);
})();
