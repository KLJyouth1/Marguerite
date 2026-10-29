import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import appSourceRaw from './App.vue?raw'
import appStoreRaw from './stores/app.ts?raw'

// Icon-rail contract (2026-10-27): the sidebar gains a persisted icon-only
// mode. These assertions pin the wiring so a refactor cannot silently drop
// the mode while the preference keeps applying.
const appSource = appSourceRaw.replace(/\r\n/g, '\n')
const storeSource = appStoreRaw.replace(/\r\n/g, '\n')
// Read the stylesheet from disk rather than via `?raw`: a `base.css?raw`
// import resolves to an empty module under vitest's CSS stubbing (the same
// reason assets/sidebar-layout.contract.test.ts reads from the filesystem).
const baseCss = readFileSync(fileURLToPath(new URL('./assets/base.css', import.meta.url)), 'utf8')
  .replace(/\r\n/g, '\n')

describe('App sidebar icon-rail contract', () => {
  it('persists the icon-rail preference in the app store', () => {
    expect(storeSource).toContain("SIDEBAR_ICON_RAIL_KEY = 'marguerite.sidebar.iconRail.v1'")
    expect(storeSource).toContain('function toggleSidebarIconRail()')
    // The actions must be exposed on the store return object.
    expect(storeSource).toContain('toggleSidebarIconRail,')
    expect(storeSource).toContain('sidebarIconRail,')
  })

  it('binds the rail class and main offset only while docked', () => {
    expect(appSource).toContain("'sidebar--icons': iconRailActive")
    expect(appSource).toContain("'main--sidebar-icons': iconRailActive")
    // Docked-only: drawer mode keeps the full column.
    expect(appSource).toMatch(
      /const iconRailActive = computed\(\s*\(\) => appStore\.sidebarIconRail && appStore\.sidebarOpen && !isSidebarDrawer\.value,/,
    )
  })

  it('hides the conversations surface in icon-rail mode', () => {
    expect(appSource).toContain('<SidebarSetupBanner v-if="!iconRailActive" />')
    expect(appSource).toContain('<SidebarConversations\n      v-if="!iconRailActive"')
  })

  it('keeps the resizable width handler out of icon-rail mode', () => {
    expect(appSource).toContain(
      'v-if="appStore.sidebarOpen && isSidebarResizable && !iconRailActive"',
    )
  })

  it('renders group anchors with a flyout and an active state', () => {
    expect(appSource).toContain('class="sidebar-icon-anchor"')
    expect(appSource).toContain('class="sidebar-icon-flyout"')
    expect(appSource).toContain('function isGroupNavActive(')
    // Section ids map to fixed glyphs (NavGroupId in router/nav.ts).
    expect(appSource).toContain("Work: 'listChecks'")
    expect(appSource).toContain("Observe: 'eye'")
  })

  it('styles the rail with tokens and offsets the main pane', () => {
    expect(baseCss).toContain('.sidebar--icons {')
    expect(baseCss).toContain('.sidebar-icon-flyout {')
    expect(baseCss).toContain('.main.main--sidebar-icons.docked {')
    // The strip width and the main offset must stay in sync.
    expect(baseCss).toContain('width: 64px;')
  })

  it('respects reduced motion in the flyout', () => {
    const reduced = baseCss.slice(baseCss.indexOf('prefers-reduced-motion: reduce'))
    expect(reduced).toContain('.sidebar-icon-flyout')
  })
})
