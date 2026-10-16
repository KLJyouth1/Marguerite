import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

// Guards the literal-string contract between webui/index.html and the Rust
// gateway's maybe_inject_index() (core/src/gateway/server.rs).
//
// That function bails out ENTIRELY — no data div, no base-href tag, no <html lang>
// — when the raw document already contains the injected data-div id, and it
// substitutes the app mount tag with str::replace, i.e. EVERY occurrence.
//
// So a stray mention of either literal in a comment — which is exactly what a
// well-meaning explanatory comment does — silently disables the whole injection.
// Every deep route (>=2 path segments) then resolves its relative assets against
// /control/<seg>/assets/..., hits the SPA fallback, receives text/html, and the
// module script is rejected by the MIME check → whole page white-screens.
//
// This cost us 9 e2e failures once (artifacts/verify-r66-batch1.log). Keep the
// literals out of index.html, and keep the mount tag appearing exactly once.

const DATA_ID = 'Marguerite-data'
const MOUNT_TAG = '<div id="app">'

const root = fileURLToPath(new URL('..', import.meta.url))
const indexPath = join(root, 'index.html')
const html = readFileSync(indexPath, 'utf8')

const failures = []

if (html.includes(DATA_ID)) {
  failures.push(
    `index.html must NOT contain the gateway-injected data-div id ("${DATA_ID}"). ` +
      'maybe_inject_index() reads its presence as "already injected" and skips the ' +
      'entire injection (locale + base href + html lang), white-screening deep routes.',
  )
}

const mountCount = html.split(MOUNT_TAG).length - 1
if (mountCount !== 1) {
  failures.push(
    `index.html must contain the app mount tag exactly once; found ${mountCount}. ` +
      'maybe_inject_index() requires exactly this literal (to inject at all) and ' +
      'substitutes EVERY occurrence of it.',
  )
}

if (failures.length > 0) {
  console.error(
    `WebUI index injection contract: ${failures.length} violation(s):\n` + failures.join('\n'),
  )
  process.exit(1)
}

console.log('WebUI index injection contract passed.')
