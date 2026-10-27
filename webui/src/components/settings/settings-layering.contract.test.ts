import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

// The desktop pet is mounted globally from App.vue at a fixed z-index, so any
// app-level overlay below it gets danced on: the pet floats over the panel and
// eats pointer events. settings-modal.spec.ts reproduced this on /settings —
// the Save button received the click 27 times and nothing happened.
//
// jsdom cannot stack real layers, so this pins the declarations instead. Read
// from disk (not `?raw`) and strip comments first: the guarded rules carry
// prose that quotes z-index values, which would otherwise satisfy the match
// even after the declaration is removed.
function loadCss(relativeUrl: string): string {
  return readFileSync(fileURLToPath(new URL(relativeUrl, import.meta.url)), 'utf8')
    .replace(/\r\n/g, '\n')
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

function ruleBody(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = css.match(new RegExp(`^${escaped}\\s*\\{([\\s\\S]*?)\\}`, 'm'))
  return match ? match[1] : ''
}

function zIndex(css: string, selector: string): number {
  const raw = ruleBody(css, selector).match(/z-index\s*:\s*(\d+)/)?.[1] ?? ''
  return raw ? Number(raw) : Number.NaN
}

const petCss = loadCss('../pet/DesktopPet.vue')
const settingsCss = loadCss('./SettingsDialog.vue')
const logsCss = loadCss('../../views/LogsView.vue')

describe('app overlay layering contract', () => {
  const petZ = zIndex(petCss, '.desktop-pet')

  it('keeps the desktop pet layer explicit', () => {
    expect(Number.isNaN(petZ), '.desktop-pet needs an explicit z-index').toBe(false)
  })

  it('renders the settings modal above the desktop pet', () => {
    const overlayZ = zIndex(settingsCss, '.settings-overlay')
    expect(Number.isNaN(overlayZ), '.settings-overlay needs an explicit z-index').toBe(false)
    expect(overlayZ).toBeGreaterThan(petZ)
    // Shared modal band (ConfirmModal 1110, view modals 1100+); anything below
    // the view drawers would also fall under toasts and lose its modality.
    expect(overlayZ).toBeGreaterThanOrEqual(1000)
  })

  it('renders the log detail drawer above the desktop pet', () => {
    const drawerZ = zIndex(logsCss, '.lg-detail-overlay')
    expect(Number.isNaN(drawerZ), '.lg-detail-overlay needs an explicit z-index').toBe(false)
    expect(drawerZ).toBeGreaterThan(petZ)
  })
})
