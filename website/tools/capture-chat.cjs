/**
 * Chat stills for the website: a real historical conversation in the brand state
 * (marguerite-night theme + 星夜 wallpaper + pet visible).
 *
 * The desktop shell dismisses the "Marguerite 正在启动" gate itself; in a plain
 * browser the gate can stay, so it is removed after a short wait. Nothing else
 * about the page is altered.
 */

const fs = require('fs');
const path = require('path');
const { chromium } = require(path.resolve(__dirname, '../../webui/node_modules/playwright'));

const BASE = process.argv[2] || 'http://127.0.0.1:18790/control';
const OUT = path.resolve(__dirname, '../assets/ui');

const BRAND_STATE = {
  'marguerite-theme': 'marguerite-night',
  'marguerite.wallpaper': JSON.stringify({ type: 'preset', id: 'marguerite' }),
  'marguerite.wallpaper.veil': '58',
  'pet.skin': 'blue',
};

async function shot(page, name, options = {}) {
  const file = path.join(OUT, `${name}.png`);
  await page.screenshot({ path: file, ...options });
  console.log(`SHOT ${name}.png ${Math.round(fs.statSync(file).size / 1024)} KB`);
}

async function dismissBootGate(page) {
  const gate = page.locator('.boot-orch');
  for (let i = 0; i < 8; i += 1) {
    if ((await gate.count()) === 0) return;
    await page.waitForTimeout(1500);
  }
  await page.evaluate(() => document.querySelector('.boot-orch')?.remove());
  await page.waitForTimeout(500);
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
    colorScheme: 'dark',
    locale: 'zh-CN',
  });
  await context.addInitScript((state) => {
    for (const [key, value] of Object.entries(state)) {
      try {
        localStorage.setItem(key, value);
      } catch {
        /* ignore */
      }
    }
  }, BRAND_STATE);

  const page = await context.newPage();
  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));

  await page.goto(`${BASE}/chat/new`, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(5200);
  await dismissBootGate(page);

  // Prefer a long conversation: more tool receipts and a finished answer.
  const candidates = [
    '系统性排查这个项目',
    '只回复两个字',
    '帮我生成一张玛格丽特公主',
  ];
  let opened = false;
  for (const text of candidates) {
    const row = page.getByText(text, { exact: false }).first();
    if ((await row.count().catch(() => 0)) === 0) continue;
    await row.click({ force: true }).catch(() => {});
    await page.waitForTimeout(4200);
    const turns = await page.locator('.chat-turn-row, .msg-ai').count().catch(() => 0);
    console.log(`open "${text}" -> turns=${turns}`);
    if (turns > 0) {
      opened = true;
      break;
    }
  }
  console.log('conversation opened:', opened);

  await page.mouse.move(720, 450);
  await page.waitForTimeout(600);
  await shot(page, 'chat');

  const composer = page.locator('textarea').first();
  if (await composer.count().catch(() => 0)) {
    await composer.click().catch(() => {});
    await composer
      .fill('把这次排查的结论整理成一页纸：根因、证据链、影响面、下一步。')
      .catch(() => {});
    await page.waitForTimeout(700);
    await shot(page, 'chat-typed');

    const box = await composer.boundingBox().catch(() => null);
    if (box) {
      const pad = 34;
      const x = Math.max(0, box.x - pad);
      const y = Math.max(0, box.y - pad);
      await shot(page, 'chat-composer', {
        clip: {
          x,
          y,
          width: Math.min(1440 - x, box.width + pad * 2),
          height: Math.min(900 - y, box.height + 86),
        },
      });
    }
  }

  await page.setViewportSize({ width: 430, height: 860 });
  await page.waitForTimeout(1400);
  await shot(page, 'chat-mobile');

  await browser.close();
})().catch((error) => {
  console.error('CAPTURE FAILED:', error);
  process.exit(1);
});
