// WCAG 对比度审计（逐主题 × 关键前景/背景组合）。
//
// 与 `check-webui-colors.mjs` 的分工：那个守卫管「不许出现裸色值」，本脚本管
// 「已经用令牌了，但配出来的对比度够不够」。两者不能互相替代 —— 全用令牌也可能
// 配出 2.1:1 的灰字压灰底。
//
// 覆盖：读取每个 `src/themes/<id>/tokens.css`（L1 令牌块）+ `src/assets/foundation.css`
// 的兜底，解析 `--token: #hex`，对下列组合算 WCAG 2.x 对比度：
//   text/bg、text/bg-surface、text-muted/bg-surface、text-dim/bg-surface、
//   accent/bg、accent-foreground/accent、danger/bg
// 阈值：正文 4.5:1（WCAG AA 小字号），非文本/大字号 3.0:1。
//
// 用法：node scripts/audit-contrast.mjs [--out 路径]
// 退出码：有低于阈值的组合 → 1（可接 CI）；否则 0。

import { readdirSync, readFileSync, existsSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const themesDir = join(root, 'src/themes')
const foundationPath = join(root, 'src/assets/foundation.css')

function parseTokens(css) {
  const out = {}
  const re = /--([a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{3,8})\s*;/g
  let m
  while ((m = re.exec(css))) out[m[1]] = m[2]
  return out
}

function hexToRgb(hex) {
  let h = hex.replace('#', '')
  if (h.length === 3) h = h.split('').map(c => c + c).join('')
  if (h.length === 8) h = h.slice(0, 6)
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]
}

function luminance([r, g, b]) {
  const f = v => {
    const s = v / 255
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

function contrast(fg, bg) {
  const a = luminance(hexToRgb(fg))
  const b = luminance(hexToRgb(bg))
  const [hi, lo] = a > b ? [a, b] : [b, a]
  return (hi + 0.05) / (lo + 0.05)
}

const PAIRS = [
  ['text', 'bg', 4.5, '正文/底色'],
  ['text', 'bg-surface', 4.5, '正文/卡片'],
  ['text-muted', 'bg-surface', 4.5, '次要文字/卡片'],
  ['text-dim', 'bg-surface', 4.5, '弱化文字/卡片'],
  ['accent', 'bg', 3.0, '强调色/底色（非文本）'],
  ['accent-foreground', 'accent', 4.5, '强调按钮文字'],
  ['danger', 'bg-surface', 4.5, '危险文字/卡片'],
]

const foundation = existsSync(foundationPath) ? parseTokens(readFileSync(foundationPath, 'utf8')) : {}
const themes = readdirSync(themesDir, { withFileTypes: true })
  .filter(d => d.isDirectory())
  .map(d => d.name)
  .filter(name => existsSync(join(themesDir, name, 'tokens.css')))
  .sort()

const rows = []
let failures = 0
for (const theme of themes) {
  const tokens = { ...foundation, ...parseTokens(readFileSync(join(themesDir, theme, 'tokens.css'), 'utf8')) }
  for (const [fgKey, bgKey, min, label] of PAIRS) {
    const fg = tokens[fgKey]
    const bg = tokens[bgKey]
    if (!fg || !bg) {
      rows.push({ theme, pair: `${fgKey}/${bgKey}`, label, ratio: null, min, ok: false, note: '缺令牌' })
      failures++
      continue
    }
    const ratio = contrast(fg, bg)
    const ok = ratio >= min
    if (!ok) failures++
    rows.push({ theme, pair: `${fgKey}/${bgKey}`, label, ratio, min, ok, note: '' })
  }
}

const outIdx = process.argv.indexOf('--out')
const outPath = outIdx > -1 ? process.argv[outIdx + 1] : null

const lines = []
lines.push('# 对比度审计（WCAG 2.x）')
lines.push('')
lines.push(`- 生成时间：${new Date().toISOString()}`)
lines.push(`- 主题数：${themes.length}`)
lines.push(`- 组合数：${rows.length}`)
lines.push(`- 未达标：${failures}`)
lines.push('')
lines.push('| 主题 | 组合 | 语义 | 实测 | 门槛 | 结果 |')
lines.push('|---|---|---|---|---|---|')
for (const r of rows) {
  lines.push(
    `| ${r.theme} | ${r.pair} | ${r.label} | ${r.ratio === null ? '—' : r.ratio.toFixed(2)} | ${r.min} | ${r.ok ? 'PASS' : `FAIL${r.note ? `（${r.note}）` : ''}`} |`,
  )
}

const report = lines.join('\n')
if (outPath) writeFileSync(outPath, `${report}\n`, 'utf8')
console.log(report)
console.log('')
console.log(failures === 0
  ? `[PASS] ${themes.length} 个主题 / ${rows.length} 组组合全部达标`
  : `[FAIL] ${failures} 组未达标`)
process.exit(failures === 0 ? 0 : 1)
