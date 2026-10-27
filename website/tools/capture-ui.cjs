/**
 * Capture real UI stills for the website (chat / settings / pet / wallpaper).
 *
 * Before each shot the page is put in the "brand state" the site promises:
 *   theme     = marguerite-night
 *   wallpaper = marguerite (专属星夜 preset), veil 62
 *   pet       = visible (bottom status bar + /pet page)
 *
 * Usage: node website/tools/capture-ui.cjs [baseUrl]
 * Requires a running gateway at http://127.0.0.1:18790.
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

async function settle(page, ms = 2600) {
  await page.waitForTimeout(ms);
  await page.evaluate(() => document.fonts && document.fonts.ready).catch(() => {});
}

async function shot(page, name, options = {}) {
  const file = path.join(OUT, `${name}.png`);
  await page.screenshot({ path: file, ...options });
  const size = fs.statSync(file).size;
  console.log(`SHOT ${name}.png ${Math.round(size / 1024)} KB`);
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
    // The site is captured as an operator surface; the gateway accepts the
    // local origin. Nothing else is faked.
  }, BRAND_STATE);

  const page = await context.newPage();
  page.on('pageerror', (error) => console.log('PAGEERROR', error.message));
  page.on('console', (msg) => {
    if (msg.type() === 'error') console.log('CONSOLE', msg.text().slice(0, 200));
  });

  // ── chat ────────────────────────────────────────────────────────────────
  await page.goto(`${BASE}/chat`, { waitUntil: 'domcontentloaded' });
  await settle(page, 3800);

  const hasMessages = await page.locator('.msg, .chat-message, [data-testid="message"]').count().catch(() => 0);
  console.log('chat messages:', hasMessages);

  if (!hasMessages) {
    // Nothing loaded yet: open the newest session from the sidebar if there is one.
    const firstSession = page.locator('aside a, .session-item, [data-testid="session-item"]').first();
    if (await firstSession.count().catch(() => 0)) {
      await firstSession.click({ timeout: 4000 }).catch(() => {});
      await settle(page, 3200);
    }
  }

  await shot(page, 'chat');

  // Composer close-up (used for the "type + send" beat in the site intro).
  const composer = page.locator('textarea').first();
  const box = await composer.boundingBox().catch(() => null);
  if (box) {
    const pad = 26;
    await shot(page, 'chat-composer', {
      clip: {
        x: Math.max(0, box.x - pad),
        y: Math.max(0, box.y - pad),
        width: Math.min(1440, box.width + pad * 2),
        height: Math.min(900, box.height + pad * 2 + 54),
      },
    });
  }

  // ── settings ────────────────────────────────────────────────────────────
  const settingsRoutes = ['settings/appearance', 'settings', 'settings/behavior'];
  for (const route of settingsRoutes) {
    await page.goto(`${BASE}/${route}`, { waitUntil: 'domcontentloaded' });
    await settle(page, 2400);
    const heading = await page.locator('h1, h2').first().textContent().catch(() => '');
    if (heading && heading.trim()) {
      await shot(page, 'settings');
      break;
    }
  }

  // ── pet + wallpaper ─────────────────────────────────────────────────────
  await page.goto(`${BASE}/pet`, { waitUntil: 'domcontentloaded' });
  await settle(page, 2400);
  await shot(page, 'pet');

  await page.goto(`${BASE}/wallpaper`, { waitUntil: 'domcontentloaded' });
  await settle(page, 2200);
  await shot(page, 'wallpaper');

  // ── portrait crop for mobile / closing slide ────────────────────────────
  await page.goto(`${BASE}/chat`, { waitUntil: 'domcontentloaded' });
  await settle(page, 2600);
  await page.setViewportSize({ width: 430, height: 860 });
  await settle(page, 1200);
  await shot(page, 'chat-mobile');

  await browser.close();
})().catch((error) => {
  console.error('CAPTURE FAILED:', error);
  process.exit(1);
});
