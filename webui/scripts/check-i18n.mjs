// i18n key-parity + English-leakage guard. Wired into `check:architecture` so a
// non-en locale can never silently drift from the en key set (vue-i18n would
// fall back to en and hide the gap) or ship untranslated English strings.
//
// en.json is the authoritative key superset. For every other locale we fail on:
//   - missing keys (present in en, absent here)
//   - extra keys (present here, absent in en)
//   - untranslated values (string identical to en and containing Latin letters)
import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

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

// Informational only: many en keys are reached dynamically (e.g. `channelStatus.${x}`,
// nav labels), so "unused" is not a defect and must never fail the guard.
const unusedCount = enKeys.filter((k) => !referenced.has(k)).length
console.log(
  `[check-i18n] source scan: ${referenced.size} literal key(s) referenced, ` +
    `${undefinedKeys.length} undefined, ${unusedCount} en key(s) reached dynamically (informational)`,
)

if (failed) {
  console.error('[check-i18n] FAILED')
  process.exit(1)
}
console.log(`[check-i18n] OK — ${OTHER_LOCALES.join(', ')} at full key parity with en, no English leakage`)
