import { describe, expect, it } from 'vitest'

import appSourceRaw from './App.vue?raw'
import { getWorkNavigationSection, getGroupedNavigation, NAV_CATEGORY, CATEGORY_ORDER } from './router/nav'
import { DEFAULT_COLLAPSED } from './app/useNavigation'

// The workspace stores .vue sources with CRLF line endings; normalize so the
// source-contract assertions can use LF-only substring expectations.
const appSource = appSourceRaw.replace(/\r\n/g, '\n')

describe('App sidebar chrome contract', () => {
  it('renders the Marguerite brand as a non-interactive lockup', () => {
    const brandStart = appSource.indexOf('<!-- Brand -->')
    const brandEnd = appSource.indexOf('<button', brandStart)
    const brandMarkup = appSource.slice(brandStart, brandEnd)

    expect(brandMarkup).toContain('<div class="sidebar-brand-lockup">')
    expect(brandMarkup).not.toContain('<router-link')
    expect(brandMarkup).not.toContain('@click')
    expect(brandMarkup).not.toContain('to="/overview"')
  })

  it('keeps project selection out of the primary sidebar controls', () => {
    const newTaskStart = appSource.indexOf('class="sidebar-new-session"')
    const workNavStart = appSource.indexOf('<router-link', newTaskStart)
    const primaryControls = appSource.slice(newTaskStart, workNavStart)

    expect(primaryControls).not.toContain('@click="openProjectPicker"')
    expect(primaryControls).not.toContain("t('workspaces.chooseProject')")
  })

  it('clears app-wide approvals for local and cross-view session deletion', () => {
    const crossViewStart = appSource.indexOf('function handleLocalSessionsDeleted')
    const crossViewEnd = appSource.indexOf('async function deleteSessions', crossViewStart)
    const crossViewDelete = appSource.slice(crossViewStart, crossViewEnd)
    expect(crossViewDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')

    const bulkStart = appSource.indexOf('async function onBulkDeleteSessions')
    const bulkEnd = appSource.indexOf('async function onDeleteSession', bulkStart)
    const bulkDelete = appSource.slice(bulkStart, bulkEnd)
    expect(bulkDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')

    const singleStart = appSource.indexOf('async function onDeleteSession')
    const singleEnd = appSource.indexOf('// Topbar approval pill', singleStart)
    const singleDelete = appSource.slice(singleStart, singleEnd)
    expect(singleDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')
  })

  it('bounds automatic sidebar RPCs after chat bootstrap admission', () => {
    expect(appSource).toContain('useSessions(\n  optionalSessionRpcCallOptions,\n)')
    expect(appSource).toContain('useAgentOptions(optionalSessionRpcCallOptions)')
    expect(appSource).toContain(
      'useSessionListSubscription({\n  rpc: rpcStore,\n  callOptions: optionalSessionRpcCallOptions,',
    )
  })

  it('keeps the primary rail to nine items or fewer', () => {
    const workNav = getWorkNavigationSection()
    expect(workNav.length).toBeLessThanOrEqual(9)
    expect(workNav.length).toBeGreaterThan(0)
  })

  it('standardizes the sidebar into core / governance / lab three-tier grouping', () => {
    // Unified with the GUI sidebar (核心 / 治理 / 实验室). Every emitted group
    // must carry a category, and the category taxonomy must be total.
    const groups = getGroupedNavigation()
    for (const group of groups) {
      expect(NAV_CATEGORY[group.id]).toBe(group.category)
    }
    // The three categories are all real, in stable display order.
    const emittedCategories = new Set(groups.map((group) => group.category))
    for (const category of CATEGORY_ORDER) {
      expect(emittedCategories.has(category)).toBe(true)
    }
    // core = Work/Operate, governance = Observe/Settings, lab = Extras.
    expect(NAV_CATEGORY.Work).toBe('core')
    expect(NAV_CATEGORY.Operate).toBe('core')
    expect(NAV_CATEGORY.Observe).toBe('governance')
    expect(NAV_CATEGORY.Settings).toBe('governance')
    expect(NAV_CATEGORY.Extras).toBe('lab')
  })

  it('collapses the lab group by default and isolates entertainment toys there', () => {
    // 实验室（Extras）默认折叠：德州扑克 / 游戏 / 鲸鱼娘(桌宠) / 视觉取证相关入口不占主屏。
    expect(DEFAULT_COLLAPSED).toContain('Extras')
    const lab = getGroupedNavigation().find((group) => group.category === 'lab')
    expect(lab).toBeDefined()
    const labPaths = lab!.items.map((item) => item.path)
    for (const toy of ['/poker', '/games', '/pet', '/wallpaper']) {
      expect(labPaths).toContain(toy)
    }
  })
})
