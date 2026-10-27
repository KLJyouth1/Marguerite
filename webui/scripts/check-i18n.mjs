// i18n key-parity + English-leakage guard. Wired into `check:architecture` so a
// non-en locale can never silently drift from the en key set (vue-i18n would
// fall back to en and hide the gap) or ship untranslated English strings.
//
// en.json is the authoritative key superset. For every other locale we fail on:
//   - missing keys (present in en, absent here)
//   - extra keys (present here, absent in en)
//   - untranslated values (string identical to en and containing Latin letters)
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, relative, resolve } from 'node:path'
import { walkFiles } from './lib/fs-walk.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const localesDir = resolve(here, '..', 'src', 'locales')
const OTHER_LOCALES = ['zh-Hans', 'ja', 'fr', 'de', 'es']
// English-leakage (value identical to en) is only a reliable "untranslated"
// signal for non-Latin scripts. fr/de/es legitimately share many words with
// English, so we enforce only KEY PARITY there and check leakage for zh-Hans.
const LEAKAGE_LOCALES = new Set(['zh-Hans'])

function load(name) {
  return JSON.parse(readFileSync(resolve(localesDir, `${name}.json`), 'utf8'))
}

function flatten(obj, prefix = '', out = {}) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k
    if (v && typeof v === 'object' && !Array.isArray(v)) flatten(v, key, out)
    else out[key] = v
  }
  return out
}

const enFlat = flatten(load('en'))
const enKeys = Object.keys(enFlat)
let failed = false

for (const loc of OTHER_LOCALES) {
  const flat = flatten(load(loc))
  const keys = Object.keys(flat)

  const missing = enKeys.filter((k) => !(k in flat))
  const extra = keys.filter((k) => !(k in enFlat))
  const untranslated = LEAKAGE_LOCALES.has(loc)
    ? enKeys.filter(
        (k) => k in flat && typeof flat[k] === 'string' && flat[k] === enFlat[k] && /[A-Za-z]/.test(flat[k]),
      )
    : []

  if (missing.length) {
    failed = true
    console.error(`[check-i18n] ${loc}: ${missing.length} missing key(s):\n  ${missing.join('\n  ')}`)
  }
  if (extra.length) {
    failed = true
    console.error(`[check-i18n] ${loc}: ${extra.length} extra key(s):\n  ${extra.join('\n  ')}`)
  }
  if (untranslated.length) {
    failed = true
    console.error(`[check-i18n] ${loc}: ${untranslated.length} untranslated (==en) value(s):\n  ${untranslated.join('\n  ')}`)
  }
}

// --- source references vs the en dictionary ---------------------------------
// The parity loop above compares locales against each other, so it structurally
// cannot see a key that NO dictionary defines: vue-i18n then renders the raw key
// string in the UI. That is how 24 keys (15 of them the whole Collab & SSH guide)
// shipped as visible `settings.ext.collabSsh.step1`-style text.
//
// So scan the source for *literal* t()/te() calls and require each one to exist in
// en. Only whole literal arguments are collected — concatenations, template
// literals and dynamic keys cannot be verified statically and would false-positive.
//
// The call may be bare (`t('k')` / `$t('k')`) or object-qualified
// (`i18n.global.t('k')`, `foo.t('k')`) — vue-i18n is almost always reached through
// the `i18n` singleton here. The old `(?<![\w$.])` lookbehind silently skipped every
// qualified call, which is how `chat.modelRouting.*` keys shipped undefined; allow a
// preceding `.` now (the `$` guard stays to avoid matching `$t` inside identifiers).
const SRC_DIR = resolve(here, '..', 'src')
const SRC_EXT = ['.vue', '.ts', '.tsx', '.js', '.mjs']
// Test files reference intentionally-fabricated keys (i18n.test.ts asserts the
// fallback of `totally.unknown.key`). They render no shipped UI, so keep them out of
// the "referenced but undefined" scan instead of treating fixtures as real gaps.
const TEST_FILE_RE = /\.(test|spec)\./
const CALL_RE = /(?<![\w$])\$?t[e]?\(\s*(['"])([^'"`\\]+)\1\s*[,)]/g

function sourceFiles(dir, out = []) {
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const full = resolve(dir, ent.name)
    if (ent.isDirectory()) {
      if (ent.name === 'node_modules' || ent.name === 'locales' || ent.name === 'dist') continue
      sourceFiles(full, out)
    } else if (SRC_EXT.some((e) => ent.name.endsWith(e)) && !TEST_FILE_RE.test(ent.name)) {
      out.push(full)
    }
  }
  return out
}

const referenced = new Map()
for (const file of sourceFiles(SRC_DIR)) {
  for (const m of readFileSync(file, 'utf8').matchAll(CALL_RE)) {
    const key = m[2]
    if (!referenced.has(key)) referenced.set(key, new Set())
    referenced.get(key).add(file.slice(SRC_DIR.length + 1).replace(/\\/g, '/'))
  }
}

const undefinedKeys = [...referenced.keys()].filter((k) => !(k in enFlat)).sort()
if (undefinedKeys.length) {
  failed = true
  console.error(
    `[check-i18n] ${undefinedKeys.length} key(s) referenced in source but defined in NO locale ` +
      `(the UI renders the raw key):\n` +
      undefinedKeys.map((k) => `  ${k}   <- ${[...referenced.get(k)].slice(0, 3).join(', ')}`).join('\n'),
  )
}

// vue-i18n 把**裸 `@`** 解析成 linked message（`@:key` / `@.mod:key`），
// 于是 `npx -y @modelcontextprotocol/...` 这种普通文案会在渲染时抛
// `SyntaxError: 10`，被 ErrorBoundary 捕获后整页降级。
//
// 实测（2026-10-21 逐路由巡检）：`/engine` 在 zh-Hans / ja / de 三个语言下
// 直接报错，而当时的 i18n 守卫全绿——它只查键对齐，不查消息体能否编译。
// 这条断言补的正是那个盲区：字面量 `@` 必须写成 `{'@'}`。
const rawAtPattern = /(^|[^'{])@/
const badBareAt = []
for (const file of ['en', ...OTHER_LOCALES]) {
  const flat = flatten(load(file))
  for (const [key, value] of Object.entries(flat)) {
    if (typeof value === 'string' && rawAtPattern.test(value)) badBareAt.push(`${file}:${key}`)
  }
}
if (badBareAt.length) {
  failed = true
  console.error(
    `[check-i18n] bare "@" in message body (write {'@'} instead) — vue-i18n parses it as a linked message:\n` +
      badBareAt.map((k) => `  ${k}`).join('\n'),
  )
}

// Informational only: many en keys are reached dynamically (e.g. `channelStatus.${x}`,
// nav labels), so "unused" is not a defect and must never fail the guard.
const unusedCount = enKeys.filter((k) => !referenced.has(k)).length
console.log(
  `[check-i18n] source scan: ${referenced.size} literal key(s) referenced, ` +
    `${undefinedKeys.length} undefined, ${unusedCount} en key(s) reached dynamically (informational)`,
)

// --- hardcoded CJK in <template> — ratchet, not a translation (W12.3) --------
// Everything above keeps the *catalogs* honest. None of it can see text that
// never reaches a catalog: Chinese typed straight into a .vue template renders as
// Chinese in every locale (ja/fr/de/es included). Translating the existing lines
// is a separate project — this ratchet only stops the count from growing.
//
// A "hit" is a physical line of the root <template> block that
//   - contains a CJK ideograph (\u4e00-\u9fff),
//   - is not inside an HTML comment, and
//   - calls neither t( / $t( / i18n. itself nor as part of the multi-line
//     {{ ... }} interpolation it belongs to.
// <script> and <style> are excluded by construction: only the template block is
// read. The rule is deliberately shallow so every hit can be re-checked by eye —
// e.g. a mixed line (`{{ t('k') }}中文`) is not a hit, and a CJK line inside a
// multi-line *attribute* value is judged on its own line even when a sibling
// line calls t(). In doubt it under-counts, which keeps the baseline stable
// instead of churning on refactors.
//
// The baseline is a ratchet: per file the count may fall (reported, still PASS)
// but never rise (FAIL). `--update-baseline` / I18N_CJK_UPDATE_BASELINE=1 writes
// a fresh or LOWER baseline and refuses to record an increase or a new offending
// file — fix the text instead of accepting it.
const CJK_BASELINE_PATH = resolve(here, 'i18n-cjk-baseline.json')
const WEBUI_ROOT = resolve(here, '..')
const CJK_BASELINE_LABEL = relative(WEBUI_ROOT, CJK_BASELINE_PATH).replace(/\\/g, '/')
const UPDATE_CJK_BASELINE =
  process.argv.includes('--update-baseline') || process.env.I18N_CJK_UPDATE_BASELINE === '1'
const CJK_RE = /[\u4e00-\u9fff]/
// Same shape as CALL_RE above (bare `t('k')` / `$t('k')` / `i18n.…`) but without
// requiring a literal first argument: `t(cond ? 'a' : 'b')` still routes through
// i18n, so the line it sits on is not hardcoded text.
const I18N_CALL_RE = /(?<![\w$])\$?t\(|(?<![\w$.])i18n\./

// Blank HTML comments without changing length, so offsets and line numbers stay
// valid after masking (comments must not count, and a commented-out <template>
// must not shift the nesting depth below).
function maskHtmlComments(text) {
  return text.replace(/<!--[\s\S]*?-->/g, (c) => c.replace(/[^\n]/g, ' '))
}

// Root <template> block of an SFC. Nesting is tracked instead of taking the first
// `</template>`, because `<template v-if>` slots close early and would silently
// truncate the scan. When the block never closes we scan to EOF: over-reporting a
// broken SFC is safer than skipping the file.
function templateSection(text) {
  const open = /^<template(?:\s[^>]*)?>/m.exec(text) || /<template(?:\s[^>]*)?>/.exec(text)
  if (!open) return null
  const start = open.index + open[0].length
  const firstLine = text.slice(0, start).split('\n').length
  const tagRe = /<\/?template(?=[\s>])/g
  let depth = 1
  tagRe.lastIndex = start
  for (let m = tagRe.exec(text); m; m = tagRe.exec(text)) {
    depth += m[0] === '</template' ? -1 : 1
    if (depth === 0) return { body: text.slice(start, m.index), firstLine }
  }
  return { body: text.slice(start), firstLine }
}

function lineIndexAt(lineStarts, offset) {
  let lo = 0
  let hi = lineStarts.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (lineStarts[mid] <= offset) lo = mid
    else hi = mid - 1
  }
  return lo
}

// Line indexes covered by a multi-line `{{ … }}` that calls t()/$t()/i18n. — the
// whole expression is routed through i18n even when only one of its lines does.
function multilineI18nLines(body) {
  const exempt = new Set()
  const lineStarts = [0]
  for (let i = 0; i < body.length; i += 1) if (body[i] === '\n') lineStarts.push(i + 1)
  for (const m of body.matchAll(/\{\{[\s\S]*?\}\}/g)) {
    if (!m[0].includes('\n') || !I18N_CALL_RE.test(m[0])) continue
    const first = lineIndexAt(lineStarts, m.index)
    const last = lineIndexAt(lineStarts, m.index + m[0].length)
    for (let i = first; i <= last; i += 1) exempt.add(i)
  }
  return exempt
}

function scanTemplateCjk() {
  const byFile = new Map()
  for (const file of walkFiles(SRC_DIR, /\.vue$/)) {
    const rel = relative(WEBUI_ROOT, file).replace(/\\/g, '/')
    const block = templateSection(maskHtmlComments(readFileSync(file, 'utf8')))
    if (!block) continue
    const exempt = multilineI18nLines(block.body)
    const hits = []
    block.body.split('\n').forEach((line, idx) => {
      if (!CJK_RE.test(line) || exempt.has(idx) || I18N_CALL_RE.test(line)) return
      hits.push({ line: block.firstLine + idx, text: line.trim() })
    })
    if (hits.length) byFile.set(rel, hits)
  }
  return byFile
}

// null = absent, 'invalid' = unparsable/ill-formed (both FAIL: an unreadable
// baseline must never silently disable the guard).
function readCjkBaseline() {
  if (!existsSync(CJK_BASELINE_PATH)) return null
  try {
    const parsed = JSON.parse(readFileSync(CJK_BASELINE_PATH, 'utf8'))
    if (!parsed || typeof parsed.files !== 'object' || Array.isArray(parsed.files)) {
      throw new Error('expected a top-level "files" object')
    }
    const files = {}
    for (const [rel, n] of Object.entries(parsed.files)) {
      if (!Number.isInteger(n) || n < 0) throw new Error(`"${rel}" must map to a non-negative integer`)
      files[rel] = n
    }
    return files
  } catch (err) {
    console.error(`[check-i18n] cannot read ${CJK_BASELINE_LABEL}: ${err.message}`)
    return 'invalid'
  }
}

function writeCjkBaseline(files) {
  const sorted = {}
  for (const rel of Object.keys(files).sort()) sorted[rel] = files[rel]
  const totalLines = Object.values(sorted).reduce((sum, n) => sum + n, 0)
  const payload = {
    generatedAt: new Date().toISOString(),
    generatedBy: 'node scripts/check-i18n.mjs --update-baseline',
    note:
      'Ratchet: hardcoded CJK lines per root <template> (lines that never call t()/$t()/i18n.). ' +
      'check-i18n.mjs FAILs when a file grows; lower counts are reported but never required. ' +
      'Regenerate with --update-baseline, which only lowers/creates and refuses to raise.',
    totalLines,
    files: sorted,
  }
  writeFileSync(CJK_BASELINE_PATH, `${JSON.stringify(payload, null, 2)}\n`, 'utf8')
  console.log(
    `[check-i18n] baseline written to ${CJK_BASELINE_LABEL}: ` +
      `${Object.keys(sorted).length} file(s), ${totalLines} line(s)`,
  )
}

function describeHits(hits) {
  const shown = hits.slice(0, 40).map((h) => `L${h.line}`).join(', ')
  const lineNumbers = hits.length > 40 ? `${shown}, … (+${hits.length - 40})` : shown
  const sample = hits.length <= 6 ? hits : hits.slice(-3)
  return [`hit line(s): ${lineNumbers}`, ...sample.map((h) => `L${h.line} ${h.text.slice(0, 120)}`)]
}

const cjkByFile = scanTemplateCjk()
const cjkTotal = [...cjkByFile.values()].reduce((sum, hits) => sum + hits.length, 0)
const cjkBaseline = readCjkBaseline()

if (UPDATE_CJK_BASELINE) {
  if (cjkBaseline === 'invalid') {
    // An unreadable baseline must never be "repaired" by rewriting it from the
    // current tree: that would silently raise the ratchet to today's count.
    failed = true
    console.error(
      `[check-i18n] refusing to write ${CJK_BASELINE_LABEL}: the existing baseline is unreadable ` +
        '(fix or delete it by hand, then re-run --update-baseline).',
    )
  } else {
    const next = {}
    const refused = []
    for (const [rel, hits] of cjkByFile) {
      const base = cjkBaseline && cjkBaseline !== 'invalid' ? cjkBaseline[rel] : undefined
      if (base === undefined) {
        // First generation records everything; later runs must not adopt a new
        // offender, otherwise the ratchet could be reset by re-running the flag.
        if (cjkBaseline && cjkBaseline !== 'invalid') {
          refused.push([`${rel}: NEW file with ${hits.length} hardcoded line(s)`, hits])
        } else {
          next[rel] = hits.length
        }
      } else if (hits.length > base) {
        // Nothing is written when anything is refused, so the old number stands.
        refused.push([`${rel}: baseline ${base} -> current ${hits.length} (+${hits.length - base})`, hits])
      } else if (hits.length > 0) {
        next[rel] = hits.length
      }
    }
    if (refused.length) {
      failed = true
      console.error(
        '[check-i18n] --update-baseline refuses to RAISE a baseline (this is a ratchet — ' +
          'route new text through t()/$t() instead of recording it):\n' +
          refused
            .map(([head, hits]) => `  ${head}\n    ${describeHits(hits).join('\n    ')}`)
            .join('\n') +
          `\n  ${CJK_BASELINE_LABEL} left untouched.`,
      )
    } else {
      writeCjkBaseline(next)
    }
  }
} else if (cjkBaseline === null || cjkBaseline === 'invalid') {
  failed = true
  console.error(
    `[check-i18n] hardcoded-CJK baseline missing or unreadable (${CJK_BASELINE_LABEL}).\n` +
      `  ${cjkTotal} hardcoded CJK line(s) in ${cjkByFile.size} template(s) are currently untracked.\n` +
      '  generate it with: node scripts/check-i18n.mjs --update-baseline',
  )
} else {
  const grown = []
  const shrunk = []
  for (const [rel, hits] of cjkByFile) {
    const base = cjkBaseline[rel] ?? 0
    if (hits.length > base) grown.push({ rel, base, hits })
    else if (hits.length < base) shrunk.push({ rel, base, now: hits.length })
  }
  for (const [rel, base] of Object.entries(cjkBaseline)) {
    if (!cjkByFile.has(rel) && base > 0) shrunk.push({ rel, base, now: 0 })
  }
  if (grown.length) {
    failed = true
    console.error(
      `[check-i18n] hardcoded CJK in <template> grew in ${grown.length} file(s) — ` +
        'route it through t()/$t() (vue-i18n renders it verbatim in ja/fr/de/es):\n' +
        grown
          .map(
            ({ rel, base, hits }) =>
              `  ${rel}: baseline ${base} -> current ${hits.length} (+${hits.length - base})\n` +
              `    ${describeHits(hits).join('\n    ')}`,
          )
          .join('\n'),
    )
  }
  const baseTotal = Object.values(cjkBaseline).reduce((sum, n) => sum + n, 0)
  const trend = grown.length
    ? `— ${grown.length} file(s) above baseline (FAIL)`
    : shrunk.length
      ? '— below baseline (PASS); run `node scripts/check-i18n.mjs --update-baseline` to lock it in'
      : '— no growth'
  console.log(
    `[check-i18n] hardcoded CJK in <template>: ${cjkTotal} line(s) in ${cjkByFile.size} file(s), ` +
      `baseline ${baseTotal} ${trend}`,
  )
  if (shrunk.length) {
    for (const { rel, base, now } of shrunk.slice(0, 10)) {
      console.log(`  ${rel}: baseline ${base} -> current ${now} (-${base - now})`)
    }
    if (shrunk.length > 10) console.log(`  … and ${shrunk.length - 10} more file(s)`)
  }
}

if (failed) {
  console.error('[check-i18n] FAILED')
  process.exit(1)
}
console.log(`[check-i18n] OK — ${OTHER_LOCALES.join(', ')} at full key parity with en, no English leakage`)
