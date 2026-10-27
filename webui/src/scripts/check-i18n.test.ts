// @vitest-environment node
// The hardcoded-CJK ratchet lives in a standalone node script, so exercise it as
// a CLI against a throwaway tree: `--update-baseline` must never "repair" an
// unreadable baseline by recording today's count (that would silently raise the
// ratchet and bypass the guard).
import { spawnSync } from 'node:child_process'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, describe, expect, it } from 'vitest'

const WEBUI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const SCRIPT = join(WEBUI_ROOT, 'scripts', 'check-i18n.mjs')
const FS_WALK = join(WEBUI_ROOT, 'scripts', 'lib', 'fs-walk.mjs')

const tempRoots: string[] = []

function scaffold(baseline: string): string {
  const root = mkdtempSync(join(tmpdir(), 'check-i18n-'))
  tempRoots.push(root)
  mkdirSync(join(root, 'scripts', 'lib'), { recursive: true })
  mkdirSync(join(root, 'src', 'locales'), { recursive: true })
  mkdirSync(join(root, 'src', 'components'), { recursive: true })
  cpSync(SCRIPT, join(root, 'scripts', 'check-i18n.mjs'))
  cpSync(FS_WALK, join(root, 'scripts', 'lib', 'fs-walk.mjs'))

  const en = { workspaces: { title: 'Projects' } }
  const translations: Record<string, unknown> = {
    'zh-Hans': { workspaces: { title: '项目' } },
    ja: { workspaces: { title: 'プロジェクト' } },
    fr: { workspaces: { title: 'Projets' } },
    de: { workspaces: { title: 'Projekte' } },
    es: { workspaces: { title: 'Proyectos' } },
  }
  writeFileSync(join(root, 'src', 'locales', 'en.json'), JSON.stringify(en, null, 2))
  for (const [locale, messages] of Object.entries(translations)) {
    writeFileSync(join(root, 'src', 'locales', `${locale}.json`), JSON.stringify(messages, null, 2))
  }
  writeFileSync(
    join(root, 'src', 'components', 'Hardcoded.vue'),
    '<template>\n  <p>写死的中文</p>\n</template>\n',
  )
  writeFileSync(join(root, 'scripts', 'i18n-cjk-baseline.json'), baseline)
  return root
}

function run(root: string) {
  return spawnSync(process.execPath, [join(root, 'scripts', 'check-i18n.mjs'), '--update-baseline'], {
    encoding: 'utf8',
    timeout: 30_000,
  })
}

afterEach(() => {
  for (const root of tempRoots.splice(0)) {
    try {
      rmSync(root, { recursive: true, force: true })
    } catch {
      // Best-effort cleanup; a leaked temp dir is not a test failure.
    }
  }
})

describe('check-i18n.mjs --update-baseline', () => {
  it('refuses to write when the existing baseline is unreadable', () => {
    const invalid = '{ "files": { "src/components/Hardcoded.vue": "one" } }'
    const root = scaffold(invalid)
    const baselinePath = join(root, 'scripts', 'i18n-cjk-baseline.json')

    const result = run(root)

    expect(result.status).toBe(1)
    expect(readFileSync(baselinePath, 'utf8')).toBe(invalid)
    expect(result.stderr).toContain('refusing to write')
    expect(result.stderr).toContain('baseline is unreadable')
  })

  it('still lowers a readable baseline to the current count', () => {
    const root = scaffold('{ "files": { "src/components/Hardcoded.vue": 5 } }')
    const baselinePath = join(root, 'scripts', 'i18n-cjk-baseline.json')

    const result = run(root)

    expect(result.stderr).not.toContain('refusing to write')
    expect(result.status).toBe(0)
    const written = JSON.parse(readFileSync(baselinePath, 'utf8'))
    expect(written.files).toEqual({ 'src/components/Hardcoded.vue': 1 })
    expect(written.totalLines).toBe(1)
  })
})
