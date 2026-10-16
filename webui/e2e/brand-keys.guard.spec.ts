import { test, expect } from '@playwright/test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

// Regression guard for the OpenSquilla → Marguerite rebrand.
//
// The e2e tree drifted badly once the product renamed its browser-storage keys:
// ten specs still asserted `opensquilla.*` keys that nothing writes any more, so
// they failed for a reason that had nothing to do with the behaviour under test
// and quietly disqualified e2e as an acceptance gate. Worse, the *write*-side
// ones kept passing by accident — `migrateLegacyStorageKeys()` rewrites a legacy
// key to the branded one at boot, so a spec that seeded the old name was really
// exercising the migration shim rather than the flag it meant to pin. Both
// failure modes are silent, so pin them here instead of relying on review.
//
// This spec deliberately uses no Playwright fixtures: it is a pure filesystem
// lint, so it runs green with no gateway, no browser and no network.

const E2E_ROOT = dirname(fileURLToPath(import.meta.url))
const SELF = 'brand-keys.guard.spec.ts'

// Retired spellings. Anything matched here has a branded replacement in src/ and
// is a genuine bug in the spec, not a stylistic nit.
const RETIRED = [
  {
    // Every dotted storage key moved to the `marguerite.` prefix, as did the
    // share-export footer caption (`opensquilla.ai` → `marguerite.ai`).
    pattern: /opensquilla\.[A-Za-z]/g,
    hint: "use the 'marguerite.' prefix (see src/utils/storageMigration.ts)",
  },
  {
    // The one non-dotted key the migration also rewrites.
    pattern: /opensquilla-theme/g,
    hint: "use 'marguerite-theme' (see src/stores/app.ts)",
  },
]

// Spellings that are still correct today. They are listed explicitly because
// they look exactly like the retired ones and have already been "fixed" by
// mistake once; each is still the live name in src/, so renaming any of them
// here would break a passing spec. Re-verify against src/ before removing an
// entry — do not extend this list just to silence a failure.
//
//   opensquilla-gateway         shipped gateway binary name
//   opensquilla-bgm             storage key, 25 uses in src/
//   opensquilla-last-route      storage key, still legacy-named in src/
//   opensquilla_active_session  storage key, still legacy-named in src/
//   OPENSQUILLA_*               environment variables
//   opensquilla<Identifier>     window probes injected by the app under test
//
// `x-opensquilla-session-key` used to be listed here too. It is **no longer
// live in src/**: the session-key request header was branded to
// `x-Marguerite-session-key` (see src/utils/chat/attachmentAccess.ts), while
// pyengine kept reading only the legacy spelling — every session-scoped
// attachment/artifact download then resolved to "no session" and answered 404.
// The gateway now accepts both, and the check below pins the two sides
// together so a future rename cannot silently break downloads again.

function collectSpecs(dir: string): string[] {
  const found: string[] = []
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) {
      found.push(...collectSpecs(full))
    } else if (entry.endsWith('.ts') && entry !== SELF) {
      found.push(full)
    }
  }
  return found
}

/** Source files under src/ that could send a request header (skips tests). */
function collectSources(dir: string): string[] {
  const found: string[] = []
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) {
      found.push(...collectSources(full))
    } else if (
      (entry.endsWith('.ts') || entry.endsWith('.vue'))
      && !/\.(test|spec)\.ts$/.test(entry)
    ) {
      found.push(full)
    }
  }
  return found
}

test('e2e specs reference no retired OpenSquilla brand keys', () => {
  const violations: string[] = []

  for (const file of collectSpecs(E2E_ROOT)) {
    const lines = readFileSync(file, 'utf8').split(/\r?\n/)
    lines.forEach((line, index) => {
      for (const { pattern, hint } of RETIRED) {
        pattern.lastIndex = 0
        if (!pattern.test(line)) continue
        const where = `${relative(E2E_ROOT, file)}:${index + 1}`
        violations.push(`${where}  ${line.trim()}\n      → ${hint}`)
      }
    })
  }

  expect(
    violations,
    `Retired OpenSquilla brand keys found in e2e specs. The product writes the\n`
      + `branded key, so these assertions test nothing (reads) or silently exercise\n`
      + `the legacy-key migration shim (writes):\n\n  `
      + violations.join('\n  ')
      + '\n',
  ).toEqual([])
})

// The session-key request header is a *wire* contract with the gateway, not a
// storage key: src/ sends it, pyengine reads it (plus the legacy spelling).
// Renaming one side only is invisible to every other check — the app keeps
// working against a mocked route while real downloads 404 — so pin the spelling
// the specs assert to the spelling src/ actually sends.
test('e2e specs assert the session header src actually sends', () => {
  // Discover the senders by scanning src/ instead of hard-coding paths: the
  // header was already moved once, and a stale path here would turn this guard
  // into a no-op (or a confusing ENOENT) exactly when it matters.
  const SRC_ROOT = join(E2E_ROOT, '..', 'src')
  const live = new Set<string>()
  for (const file of collectSources(SRC_ROOT)) {
    const text = readFileSync(file, 'utf8')
    for (const match of text.matchAll(/['"](x-[A-Za-z-]*session-key)['"]/g)) {
      live.add(match[1].toLowerCase())
    }
  }
  expect(
    [...live],
    'No session-key header literal found anywhere in src/ — if the console really '
      + 'stopped sending one, delete this guard; otherwise re-point SRC_ROOT.',
  ).not.toHaveLength(0)

  const mismatches: string[] = []
  for (const file of collectSpecs(E2E_ROOT)) {
    const lines = readFileSync(file, 'utf8').split(/\r?\n/)
    lines.forEach((line, index) => {
      for (const match of line.matchAll(/x-[A-Za-z-]*session-key/g)) {
        if (live.has(match[0].toLowerCase())) continue
        mismatches.push(
          `${relative(E2E_ROOT, file)}:${index + 1}  ${line.trim()}\n`
            + `      → src/ sends ${[...live].join(', ')}`,
        )
      }
    })
  }

  expect(
    mismatches,
    'These specs assert a session header the console never sends, so they either\n'
      + 'fail against the real gateway or silently pass against a mocked route:\n\n  '
      + mismatches.join('\n  ')
      + '\n',
  ).toEqual([])
})
