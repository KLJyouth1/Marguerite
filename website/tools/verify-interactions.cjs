/**
 * Interaction proof for the upgraded site:
 *   - the film starts from a real click, audio time drives the timeline
 *   - acts switch on the analysed beat grid (no arbitrary timers)
 *   - typing / cursor / diff cells / brand bloom render
 *   - the demo composer really types + streams
 *
 * Usage: node tools/verify-interactions.cjs
 */

const path = require('path');
const { chromium } = require(path.resolve(__dirname, '../../webui/node_modules/playwright'));

const URL = 'file:///' + path.resolve(__dirname, '../index.html').replace(/\\/g, '/');

(async () => {
  const browser = await chromium.launch({
    args: ['--autoplay-policy=no-user-gesture-required'],
  });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));

  await page.goto(URL, { waitUntil: 'load' });
  await page.waitForTimeout(600);

  // ── start the film by clicking the cover (real gesture) ─────────────────
  await page.locator('#film').scrollIntoViewIfNeeded();
  await page.waitForTimeout(500);
  await page.locator('[data-cover]').click();
  await page.waitForTimeout(9000);

  const state1 = await page.evaluate(() => {
    const audio = document.getElementById('soundtrack');
    const active = document.querySelector('.act.is-active');
    const stage = document.getElementById('playerStage');
    return {
      paused: audio.paused,
      time: Number(audio.currentTime.toFixed(2)),
      active: active ? active.getAttribute('data-act') : null,
      bpm: stage.getAttribute('data-bpm'),
      typed: (document.querySelector('[data-typer]') || {}).textContent || '',
      clock: document.querySelector('[data-clock]').textContent,
      beats: window.MARGUERITE_BEATS ? window.MARGUERITE_BEATS.beatBpm : null,
      zoom: (document.querySelector('.act--chat .shotframe__img') || { style: {} }).style.transform || '',
    };
  });
  await page.screenshot({ path: '_cine-act1.png' });

  // ── jump through the acts by seeking the audio (same clock) ─────────────
  const jump = async (seconds, name, waitMs = 1400) => {
    await page.evaluate((t) => {
      const audio = document.getElementById('soundtrack');
      audio.currentTime = t;
    }, seconds);
    await page.waitForTimeout(waitMs);
    const snap = await page.evaluate(() => {
      const active = document.querySelector('.act.is-active');
      return {
        active: active ? active.getAttribute('data-act') : null,
        clock: document.querySelector('[data-clock]').textContent,
      };
    });
    await page.screenshot({ path: name });
    return snap;
  };

  const act2 = await jump(31, '_cine-act2.png', 1800);
  const act3 = await jump(46, '_cine-act3.png', 2200);
  const act4 = await jump(63, '_cine-act4.png', 1600);
  const act5 = await jump(74, '_cine-act5.png', 2600);

  // ── demo: real typing + send + stream ──────────────────────────────────
  await page.locator('#demo').scrollIntoViewIfNeeded();
  await page.waitForTimeout(400);
  await page.locator('[data-demo-input]').click();
  await page.locator('[data-demo-input]').type('把这次排查的结论整理成一页纸：根因、证据链、影响面、下一步。', { delay: 18 });
  await page.locator('[data-demo-send]').click();
  await page.waitForTimeout(1250);
  await page.screenshot({ path: '_demo-tool.png' });
  await page.waitForTimeout(2600);
  const demo = await page.evaluate(() => ({
    user: (document.querySelector('.demo__msg-u') || {}).textContent || '',
    tool: (document.querySelector('.demo__tool') || {}).textContent || '',
    answer: (document.querySelector('.demo__msg-a') || {}).textContent || '',
  }));
  await page.screenshot({ path: '_demo-stream.png' });

  // ── brand closing ──────────────────────────────────────────────────────
  await page.locator('#brand').scrollIntoViewIfNeeded();
  await page.waitForTimeout(2200);
  await page.screenshot({ path: '_brand-closing.png' });

  console.log(JSON.stringify({ state1, act2, act3, act4, act5, demo, errors }, null, 2));
  await browser.close();
})().catch((error) => {
  console.error('VERIFY FAILED:', error);
  process.exit(1);
});
