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
//   x-opensquilla-session-key   request header, 15 uses in src/
//   opensquilla-gateway         shipped gateway binary name
//   opensquilla-bgm             storage key, 25 uses in src/
//   opensquilla-last-route      storage key, still legacy-named in src/
//   opensquilla_active_session  storage key, still legacy-named in src/
//   OPENSQUILLA_*               environment variables
//   opensquilla<Identifier>     window probes injected by the app under test

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
