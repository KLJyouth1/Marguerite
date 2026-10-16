// Screenshot the island-game MVP route (3D + forced 2D fallback) for evidence.
import { chromium } from 'playwright'
import { mkdirSync } from 'node:fs'

const OUT = 'D:/Downloads/Marguerite/artifacts'
mkdirSync(OUT, { recursive: true })
const ts = new Date().toISOString().replace(/[:.]/g, '-')
const URL = 'http://localhost:5174/control/island-game'

async function shoot(label, initScript) {
  const browser = await chromium.launch()
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } })
  if (initScript) await page.addInitScript(initScript)
  const errors = []
  page.on('pageerror', (e) => errors.push(String(e)))
  await page.goto(URL, { waitUntil: 'networkidle' }).catch(() => {})
  await page.waitForTimeout(3500)
  const file = `${OUT}/island-game-${label}-${ts}.png`
  await page.screenshot({ path: file, fullPage: false })
  console.log('SHOT', file, 'pageerrors:', errors.length ? errors.join(' | ') : 'none')
  await browser.close()
}

await shoot('3d', undefined)
await shoot('2d-fallback', () => {
  const orig = HTMLCanvasElement.prototype.getContext
  HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
    if (typeof type === 'string' && type.includes('webgl')) return null
    return orig.call(this, type, ...rest)
  }
})
console.log('DONE', ts)
