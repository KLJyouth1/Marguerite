// @vitest-environment happy-dom
// Tests for the global top-bar mode badge (P0-2: converged on the 3×3 user mode).
//
// The badge used to keep its own five-kind vocabulary (MODE_KIND_LIST) while the
// composer rendered a 3×3 grid, so one page could claim two different modes. It
// now renders `activeUserMode` only; the runtime kind is a compatibility read and
// every switch goes through `switchUserMode`.
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import i18n, { loadLocaleMessages } from '@/i18n'

import { useActiveModeStore, userModeInfoFor, type UserModeInfo } from '@/stores/activeMode'
import { activeMode } from '@/composables/useActiveMode'
import ModeBadge from '@/components/ModeBadge.vue'

/** Resolve a canonical slot, failing loudly on a typo'd key (never `null!`). */
function slot(key: string): UserModeInfo {
  const info = userModeInfoFor(key)
  if (!info) throw new Error(`unknown user-mode slot: ${key}`)
  return info
}

const routerPush = vi.fn()
const switchMock = vi.fn().mockResolvedValue(null)

// The active-mode state lives in a Pinia store (it must be shared across route
// chunks). Create the pinia ONCE: the imported `activeMode` ref resolves its
// store through the active pinia, so recreating pinia per test would leave that
// ref bound to a stale instance and writes would never reach the component.
const pinia = createPinia()
setActivePinia(pinia)

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: routerPush }),
}))

vi.mock('@/composables/useActiveMode', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/composables/useActiveMode')>()
  return {
    ...actual,
    // Overriding the bare `switchUserMode` export is NOT enough: the component
    // consumes it through `useActiveMode()`, and the real composable closes over
    // the module-local binding. Wrap the composable itself instead.
    useActiveMode: () => ({
      ...actual.useActiveMode(),
      switchUserMode: (key: string) => switchMock(key),
    }),
  }
})

function mount(node: any): { el: HTMLElement; unmount: () => void } {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app: App = createApp(node)
  // Install the same pinia the test writes through, so component setup resolves
  // the identical store instance.
  app.use(pinia)
  app.use(i18n)
  app.mount(el)
  return { el, unmount: () => app.unmount() }
}

function menuItems(el: HTMLElement): HTMLElement[] {
  return Array.from(el.querySelectorAll('.mode-menu__item')) as HTMLElement[]
}

describe('ModeBadge', () => {
  beforeEach(() => {
    // Reset state through the store rather than swapping pinia.
    useActiveModeStore().mode = null
    routerPush.mockReset()
    switchMock.mockClear()
  })

  it('renders the 3x3 user-mode label as the single source of truth', async () => {
    activeMode.value = { kind: 'bloom', mode: 'bloom', userMode: slot('standard_doit') }

    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()

    const label = el.querySelector('.mode-badge__label')
    expect(label?.textContent).toBe('Standard · Act')
    // The dot follows the working axis, not the legacy runtime kind.
    expect(el.querySelector('.mode-badge__dot--doit')).toBeTruthy()
    expect(el.querySelector('.mode-badge__dot--bloom')).toBeFalsy()
    unmount()
  })

  it('derives the 3x3 slot from a legacy kind-only payload (compatibility read)', async () => {
    // An older gateway broadcasts `kind` without `userMode`; the badge must still
    // land on the canonical cell instead of showing a dash.
    activeMode.value = { kind: 'agent', mode: 'agent' }

    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()

    expect(el.querySelector('.mode-badge__label')?.textContent).toBe('Standard · Plan')
    unmount()
  })

  it('offers exactly the nine canonical slots and switches through switchUserMode', async () => {
    activeMode.value = { kind: 'agent', mode: 'agent', userMode: slot('standard_plan') }

    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()

    const btn = el.querySelector('.mode-badge') as HTMLButtonElement
    btn.click()
    await nextTick()

    const items = menuItems(el)
    expect(items.length).toBe(9)
    expect(items.map((i) => i.getAttribute('title')?.split(' · ')[0])).toEqual([
      'minimal_ask', 'minimal_plan', 'minimal_doit',
      'standard_ask', 'standard_plan', 'standard_doit',
      'extreme_ask', 'extreme_plan', 'extreme_doit',
    ])

    // Active cell is the payload's slot, not the legacy kind's neighbour.
    expect(items[4].getAttribute('aria-checked')).toBe('true')

    const extreme = items[8] as HTMLElement
    extreme.click()
    await nextTick()
    expect(switchMock).toHaveBeenCalledWith('extreme_doit')
    unmount()
  })

  it('provides a Control Center entry inside the menu', async () => {
    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()
    ;(el.querySelector('.mode-badge') as HTMLElement).click()
    await nextTick()
    const foot = el.querySelector('.mode-menu__foot') as HTMLElement
    expect(foot).toBeTruthy()
    foot.click()
    await nextTick()
    expect(routerPush).toHaveBeenCalledWith('/control-center')
    unmount()
  })

  it('falls back to a dash when no mode is loaded', async () => {
    activeMode.value = null
    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()
    const label = el.querySelector('.mode-badge__label')
    expect(label?.textContent).toBe('—')
    unmount()
  })

  it('normalizes a legacy underscore kind to the canonical slot', async () => {
    // core used to broadcast `omni_transcend`; the badge must still resolve it.
    activeMode.value = { kind: 'omni_transcend', mode: 'omni_transcend' }
    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()
    expect(el.querySelector('.mode-badge__label')?.textContent).toBe('Extreme · Act')
    unmount()
  })

  it('keeps the technical runtime kind in the tooltip for log cross-checks', async () => {
    activeMode.value = { kind: 'bloom', mode: 'bloom', userMode: slot('standard_doit') }
    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()
    const title = el.querySelector('.mode-badge')?.getAttribute('title') ?? ''
    expect(title).toContain('standard_doit')
    expect(title).toContain('bloom')
    expect(title).toContain('Switch modes from the top-bar badge')
    unmount()
  })

  it('right-click cycles through the legacy five stops but writes a user-mode key', async () => {
    activeMode.value = { kind: 'agent', mode: 'agent', userMode: slot('standard_plan') }
    const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
    await nextTick()
    ;(el.querySelector('.mode-badge') as HTMLButtonElement).dispatchEvent(
      new MouseEvent('contextmenu', { bubbles: true, cancelable: true }),
    )
    await nextTick()
    // agent → next legacy stop is ensemble → minimal_doit
    expect(switchMock).toHaveBeenCalledWith('minimal_doit')
    unmount()
  })

  it('shows the zh-Hans labels for every slot', async () => {
    await loadLocaleMessages('zh-Hans')
    i18n.global.locale.value = 'zh-Hans'
    try {
      activeMode.value = { kind: 'bloom', mode: 'bloom', userMode: slot('standard_doit') }
      const { el, unmount } = mount({ components: { ModeBadge }, template: '<ModeBadge />' })
      await nextTick()
      expect(el.querySelector('.mode-badge__label')?.textContent).toBe('标准·执行')

      ;(el.querySelector('.mode-badge') as HTMLElement).click()
      await nextTick()
      const labels = menuItems(el).map((i) => i.querySelector('.mode-menu__item-label')?.textContent)
      expect(labels).toEqual([
        '极简·咨询', '极简·方案', '极简·执行',
        '标准·咨询', '标准·方案', '标准·执行',
        '极致·咨询', '极致·方案', '极致·执行',
      ])
      unmount()
    } finally {
      i18n.global.locale.value = 'en'
    }
  })
})
