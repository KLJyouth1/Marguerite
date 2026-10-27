// check-tokens-sync.mjs
// ---------------------------------------------------------------------------
// M1/E1 · 设计令牌三端同步校验 —— 单一来源 design/tokens.json
//
// 消费方（均为 tokens.json 的派生文件，禁止在派生文件里修改品牌令牌值）：
//   - WebUI : webui/src/themes/dark/tokens.css  (--accent / --accent-secondary …)
//   - TUI   : core/src/tui/workbench/theme.rs   (DEEP_SPACE.brand = Color::Rgb …)
//
// ⚠️ 2026-10-21：原第三端 **GUI（gui/src/theme.css）已归档**。
// `gui/src` 是 React 前端，从未进入安装包（`tauri.conf.json` 的 `url` 指向网关
// `http://localhost:18790/control/`，`bundle.resources` 也不含 `gui/dist`），
// 界面收敛为 webui（Vue）单一实现，源码归档在 `docs/legacy/gui-react-2026-10-21/`。
//
// 由此产生一个**必须登记、不能静默忽略**的缺口（见下方 ARCHIVED_GAPS）：
//   · `semantic.*`（success/warning/error/info 四色）：GUI 是唯一消费者。
//     WebUI 用的是**另一套命名**（--ok / --danger / --info / --queued），
//     语义部分对应但值域与命名都不同，故不能机械迁移。
//   · `typography.fontSize`（px 绝对值 13/12/14/16/18/22）：GUI 是唯一消费者。
//     WebUI foundation 用 rem 相对值（0.875/1/1.1875/1.625/2 rem）且多 `--fs-xs`、
//     少 `base` —— 两套量纲不同，同样不可迁移。
// 这两段现在是「单一真源里暂时无人消费的登记项」。要做的是**设计系统统一**（独立项目），
// 不是在这里删掉检查假装它们同步。
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

// ---- WebUI 各主题语义色（2026-10-22 登记）----
//
// 背景：这批令牌此前**完全没有登记也没有守卫** —— 10 个主题 × 5 个语义色 = 50 个值，
// 任何人随手改 dark/tokens.css 的 --danger 都不会有门禁报错。而"深色主题下错误色
// 对比度不足"正是这类无守卫改动最容易造成的缺陷。
//
// 判据三条（缺一条就会出现"偷偷绕开登记"）：
//   a) 已登记主题：CSS 里的值必须与 semanticByTheme 逐字相等；
//   b) CSS 里出现了 semanticByTheme 没有的语义令牌 → 未登记，报错；
//   c) themes/ 下有主题目录但 semanticByTheme 没有对应条目 → 主题未登记，报错。
//
// 刻意**不比对不同主题之间的值**：10 个主题的语义色是为各自明度/色调调过的
// （arctic 低饱和、crt-green 荧光、ink 水墨去饱和），强行统一会毁掉视觉表达。
// 要统一的是登记与判据，不是颜色值。
const SEMANTIC_TOKENS = ['ok', 'warn', 'danger', 'info', 'queued']
const THEMES_DIR = 'webui/src/themes'
const semanticByTheme = tokens.webuiAdapted?.semanticByTheme ?? {}

let unregisteredTokenCount = 0
for (const theme of Object.keys(semanticByTheme)) {
  if (theme === 'comment') continue
  const cssPath = `${THEMES_DIR}/${theme}/tokens.css`
  let themeCss
  try {
    themeCss = read(cssPath)
  } catch {
    check(`WebUI(${theme}) 主题文件存在`, false, `读取失败：${cssPath}`)
    continue
  }
  for (const [token, entry] of Object.entries(semanticByTheme[theme])) {
    const actual = cssVar(themeCss, `--${token}`)
    check(
      `WebUI(${theme}) --${token} == tokens.webuiAdapted.semanticByTheme.${theme}.${token}`,
      actual === entry.value,
      `登记 ${entry.value}，实际 ${actual ?? '(缺失)'}`,
    )
    // canonical 必须指向真实存在的品牌字段（防止写了映射却没对应值）
    if (entry.canonical) {
      const [, brandKey] = entry.canonical.split('.')
      check(
        `WebUI(${theme}) --${token} 的 canonical 指向真实字段 (${entry.canonical})`,
        Boolean(tokens.semantic?.[brandKey]?.value),
        `tokens.semantic.${brandKey} 不存在`,
      )
    }
  }
}

// b) 反向：CSS 里出现但未登记的语义令牌
//
// 判据是**精确匹配**，不是前缀匹配。最初写成"允许 --ok-* 派生"，实测被
// `--danger-alt` 直接绕过 —— 派生规则成了后门。现在：任何名字里含
// ok/warn/danger/info/queued 的自定义属性，必须**恰好等于**已登记的名字之一。
//
// 如实说明局限（不假装覆盖）：本判据只扫"名字像语义色"的令牌。若有人新增
// `--notice: #fff` 这类完全不同名的语义概念，本闸门看不见 —— 那需要全量令牌
// 清单（42 令牌 × 10 主题），属独立批次。此处不虚报覆盖率。
for (const theme of Object.keys(semanticByTheme)) {
  if (theme === 'comment') continue
  let themeCss
  try {
    themeCss = read(`${THEMES_DIR}/${theme}/tokens.css`)
  } catch {
    continue
  }
  const registered = new Set(Object.keys(semanticByTheme[theme]))
  // 只匹配属性名（冒号前），避免把注释或值里的字样带进来
  for (const m of themeCss.matchAll(/(?:^|[^\w-])--([a-z0-9-]*(?:ok|warn|danger|info|queued)[a-z0-9-]*)\s*:/g)) {
    const name = m[1]
    if (registered.has(name)) continue
    unregisteredTokenCount++
    check(
      `WebUI(${theme}) 语义令牌 --${name} 已登记`,
      false,
      '名字含语义词但不在 semanticByTheme 登记清单里 —— 新增语义色必须先登记（不允许用 --danger-alt 之类派生绕过）',
    )
  }
}

// c) 反向：主题目录存在但未登记
try {
  const { readdirSync } = await import('node:fs')
  const dirs = readdirSync(path.join(root, THEMES_DIR), { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
  for (const d of dirs) {
    const hasCss = (() => {
      try {
        read(`${THEMES_DIR}/${d}/tokens.css`)
        return true
      } catch {
        return false
      }
    })()
    if (!hasCss) continue
    check(
      `主题 ${d} 已登记于 semanticByTheme`,
      Object.prototype.hasOwnProperty.call(semanticByTheme, d),
      `${THEMES_DIR}/${d}/tokens.css 存在但 semanticByTheme 无该主题 —— 新主题必须登记其语义色`,
    )
  }
} catch {
  /* themes 目录不存在时不判失败（结构变化不应误判） */
}

// ---- TUI 端 ----
const tuiRgb = tuiRs.match(/brand:\s*Color::Rgb\((\d+),\s*(\d+),\s*(\d+)\)/)
const brandRgb = hexToRgb(tokens.brand.blue.value)
check('TUI theme.rs DEEP_SPACE.brand == tokens.brand.blue',
  tuiRgb !== null && tuiRgb[1] === String(brandRgb[0]) && tuiRgb[2] === String(brandRgb[1]) && tuiRgb[3] === String(brandRgb[2]),
  tuiRgb ? `expected rgb(${brandRgb.join(', ')}) == #${tokens.brand.blue.value}, got rgb(${tuiRgb[1]}, ${tuiRgb[2]}, ${tuiRgb[3]})` : 'brand rgb 未匹配')

// ---- 字体排印令牌（M3/E2，SSOT -> design/tokens.json -> typography）----
const fF = tokens.typography.fontFamily
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

// ---- 归档守卫：GUI 前端不得悄悄回来 ----
//
// 双前端是本仓付过代价的债（两套 token 纪律 / 两套测试 / 两套导航分类法，
// 而其中一套用户永远看不到）。若 `gui/src` 被误加回来，本闸门必须立刻报错，
// 而不是"多扫一个目录、"结果更绿"。
import { existsSync } from 'node:fs'
check('guard: gui/src 前端源码不得重现（已归档至 docs/legacy/gui-react-2026-10-21/）',
  !existsSync(path.join(root, 'gui/src')),
  'gui/src 存在 —— 双前端已废止，请勿恢复；前端只在 webui/（Vue）维护')

// ---- 已归档的缺口登记（**不是通过项**，但也不是失败：它们在等待设计系统统一）----
const ARCHIVED_GAPS = [
  // 2026-10-22：semantic.* 一项已销账 —— 它不再是"无消费者"：
  // 现在通过 webuiAdapted.semanticByTheme.<theme>.<token>.canonical 建立到
  // WebUI 各主题的映射，并由本闸门逐条断言 canonical 指向真实字段。
  'tokens.typography.fontSize — 原 GUI px 绝对值独有；WebUI 用 rem 相对值，量纲不同',
]

// 汇总
let failed = 0
console.log('=== design/tokens.json 端点同步校验（WebUI + TUI；GUI 已归档）===')
for (const c of checks) {
  console.log(`${c.ok ? '[PASS]' : '[FAIL]'} ${c.name}${c.ok ? '' : '  -> ' + c.detail}`)
  if (!c.ok) failed++
}
if (ARCHIVED_GAPS.length) {
  console.log('')
  console.log('[ARCHIVED] 以下真源字段当前**无运行时消费者**（GUI 归档所致），登记待办而非静默删除：')
  for (const g of ARCHIVED_GAPS) console.log(`  · ${g}`)
}
console.log('')
console.log(
  failed === 0
    ? '闸门通过：design/tokens.json 与 WebUI/TUI 派生值同步；GUI 段已按归档登记。'
    : `未通过：${failed} 项漂移，请同步派生值后再声称完成。`,
)
process.exit(failed === 0 ? 0 : 1)
