// check-tokens-sync.mjs
// ---------------------------------------------------------------------------
// M1/E1 · 设计令牌三端同步校验 —— 单一来源 design/tokens.json
//
// 消费方（均为 tokens.json 的派生文件，禁止在派生文件里修改品牌令牌值）：
//   - GUI   : gui/src/theme.css          (--accent-primary / --accent-secondary / --status-*)
//   - WebUI : webui/src/themes/dark/tokens.css  (--accent / --accent-secondary …)
//   - TUI   : core/src/tui/workbench/theme.rs   (DEEP_SPACE.brand = Color::Rgb …)
//
// 铁律：任何「品牌/语义令牌」在屏幕上可不一致，但它们的 *不同值* 必须能在
// tokens.json 中找到登记（如 WebUI dark 的浅蓝加速适应值）。改一处 tokens.json
// 后，本脚本会驱动三端派生值同步，任何漂移 → 非零退出 → verify.sh 标红。
// ---------------------------------------------------------------------------
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const read = (p) => readFileSync(path.join(root, p), 'utf8')

const tokens = JSON.parse(read('design/tokens.json'))
const guiCss = read('gui/src/theme.css')
const webuiDark = read('webui/src/themes/dark/tokens.css')
const webuiFoundation = read('webui/src/assets/foundation.css')
const tuiRs = read('core/src/tui/workbench/theme.rs')

const hexToRgb = (hex) => {
  const h = hex.replace('#', '')
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]
}
// 取值到属性行末的 `;`（支持含引号/逗号/空格的字体栈值）
const cssVar = (css, name) => {
  const m = css.match(new RegExp(`${name}\\s*:\\s*([^;]+)\\s*;`))
  return m ? m[1].trim() : null
}
// 规范化空白，消除单/多空格差异后再比较
const norm = (s) => (s ?? '').replace(/\s+/g, ' ').trim()

const checks = []
const check = (name, ok, detail) => checks.push({ name, ok, detail })

// ---- GUI 端 ----
const guiBrandBlue = cssVar(guiCss, '--accent-primary')
check('GUI --accent-primary == tokens.brand.blue',
  guiBrandBlue === tokens.brand.blue.value,
  `expected ${tokens.brand.blue.value}, got ${guiBrandBlue}`)
check('GUI --accent-secondary == tokens.brand.orange',
  cssVar(guiCss, '--accent-secondary') === tokens.brand.orange.value,
  `expected ${tokens.brand.orange.value}, got ${cssVar(guiCss, '--accent-secondary')}`)
for (const [k, v] of [['--status-success', 'success'], ['--status-warning', 'warning'], ['--status-error', 'error'], ['--status-info', 'info']]) {
  const actual = cssVar(guiCss, k)
  check(`GUI ${k} == tokens.semantic.${v}`,
    actual === tokens.semantic[v].value,
    `expected ${tokens.semantic[v].value}, got ${actual}`)
}

// ---- WebUI 端（dark 默认运行时主题）----
check('WebUI(dark) --accent-secondary == tokens.brand.orange',
  cssVar(webuiDark, '--accent-secondary') === tokens.brand.orange.value,
  `expected ${tokens.brand.orange.value}, got ${cssVar(webuiDark, '--accent-secondary')}`)
for (const [cssName, jsonKey] of [['--accent', 'accent'], ['--accent-hover', 'accentHover'], ['--accent-deep', 'accentDeep'], ['--accent-foreground', 'accentForeground']]) {
  const expect = tokens.webuiAdapted.dark[jsonKey].value
  const actual = cssVar(webuiDark, cssName)
  check(`WebUI(dark) ${cssName} == tokens.webuiAdapted.dark.${jsonKey}`,
    actual === expect,
    `expected ${expect}, got ${actual}`)
}

// ---- TUI 端 ----
const tuiRgb = tuiRs.match(/brand:\s*Color::Rgb\((\d+),\s*(\d+),\s*(\d+)\)/)
const brandRgb = hexToRgb(tokens.brand.blue.value)
check('TUI theme.rs DEEP_SPACE.brand == tokens.brand.blue',
  tuiRgb !== null && tuiRgb[1] === String(brandRgb[0]) && tuiRgb[2] === String(brandRgb[1]) && tuiRgb[3] === String(brandRgb[2]),
  tuiRgb ? `expected rgb(${brandRgb.join(', ')}) == #${tokens.brand.blue.value}, got rgb(${tuiRgb[1]}, ${tuiRgb[2]}, ${tuiRgb[3]})` : 'brand rgb 未匹配')

// ---- 字体排印令牌（M3/E2，SSOT -> design/tokens.json -> typography）----
const fF = tokens.typography.fontFamily
check('GUI --font-sans == tokens.typography.fontFamily.sans',
  norm(cssVar(guiCss, '--font-sans')) === norm(fF.sans.value),
  `got ${cssVar(guiCss, '--font-sans')}`)
check('GUI --font-display == tokens.typography.fontFamily.display',
  norm(cssVar(guiCss, '--font-display')) === norm(fF.display.value),
  `got ${cssVar(guiCss, '--font-display')}`)
check('GUI --font-mono == tokens.typography.fontFamily.mono',
  norm(cssVar(guiCss, '--font-mono')) === norm(fF.mono.value),
  `got ${cssVar(guiCss, '--font-mono')}`)
const allGuiDisplay = [...guiCss.matchAll(/--font-display\s*:\s*([^;]+);/g)].map(m => m[1].trim())
check('GUI [data-theme=ink] --font-display == tokens.typography.fontFamily.inkDisplay',
  allGuiDisplay.length >= 2 && norm(allGuiDisplay[1]) === norm(fF.inkDisplay.value),
  `ink display got ${allGuiDisplay[1] ?? 'missing'}`)
// WebUI 端字体栈是 tokens.json 里**登记过的**适应值（webuiSans / webuiDisplay）：
// WebUI 自托管 IBM Plex Sans / Space Grotesk（assets/fonts/*.woff2），故在其派生值前
// 多了自托管族；GUI 不打包这两个字体，仍与 sans / display 一致。
// 这里仍然做**精确相等**校验（不允许自由漂移），只是把 WebUI 的期望值指到登记项上。
const webuiSansToken = fF.webuiSans ?? fF.sans
const webuiDisplayToken = fF.webuiDisplay ?? fF.display
check('WebUI foundation --font-sans-default == tokens.typography.fontFamily.webuiSans',
  norm(cssVar(webuiFoundation, '--font-sans-default')) === norm(webuiSansToken.value),
  `got ${cssVar(webuiFoundation, '--font-sans-default')}`)
check('WebUI foundation --font-display-default == tokens.typography.fontFamily.webuiDisplay',
  norm(cssVar(webuiFoundation, '--font-display-default')) === norm(webuiDisplayToken.value),
  `got ${cssVar(webuiFoundation, '--font-display-default')}`)
check('WebUI foundation --font-mono == tokens.typography.fontFamily.mono',
  norm(cssVar(webuiFoundation, '--font-mono')) === norm(fF.mono.value),
  `got ${cssVar(webuiFoundation, '--font-mono')}`)
for (const [cssName, key] of [['--fs-base', 'base'], ['--fs-sm', 'sm'], ['--fs-md', 'md'], ['--fs-lg', 'lg'], ['--fs-xl', 'xl'], ['--fs-2xl', '2xl']]) {
  const expect = tokens.typography.fontSize[key].value
  const actual = cssVar(guiCss, cssName)
  check(`GUI ${cssName} == tokens.typography.fontSize.${key}`, actual === expect, `got ${actual}`)
}

// 汇总
let failed = 0
console.log('=== design/tokens.json 三端同步校验 ===')
for (const c of checks) {
  console.log(`${c.ok ? '[PASS]' : '[FAIL]'} ${c.name}${c.ok ? '' : '  -> ' + c.detail}`)
  if (!c.ok) failed++
}
console.log(failed === 0 ? '闸门通过：design/tokens.json 与 GUI/WebUI/TUI 三端派生值同步。' : `未通过：${failed} 项漂移，请同步三端派生值后再声称完成。`)
process.exit(failed === 0 ? 0 : 1)
