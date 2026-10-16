// @vitest-environment happy-dom
import { createApp, nextTick, type App } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

import i18n from '@/i18n'
import CommandPalette from './CommandPalette.vue'
import { useActiveModeStore } from '@/stores/activeMode'

// The palette reads the shared active-mode state, which now lives in a Pinia
// store (it has to be shared across route chunks), so the app under test must
// have a pinia installed.
const pinia = createPinia()
setActivePinia(pinia)

const routerPush = vi.fn()
const rpcCall = vi.fn(async () => ({ sessions: [], messages: [] }))

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: routerPush }),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ call: rpcCall }),
}))

vi.mock('@/composables/useBgm', () => ({
  useBgm: () => ({
    enabled: { value: false },
    setEnabled: vi.fn(),
  }),
}))

describe('CommandPalette navigation commands', () => {
  let app: App<Element> | null = null

  afterEach(() => {
    app?.unmount()
    app = null
    document.body.innerHTML = ''
    routerPush.mockReset()
    rpcCall.mockReset()
    useActiveModeStore().mode = null
  })

  async function mountPalette() {
    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(CommandPalette, { open: true, recents: [] })
    app.use(pinia)
  app.use(i18n)
    app.mount(el)
    await nextTick()
    return el
  }

  async function search(el: Element, value: string) {
    const input = el.querySelector<HTMLInputElement>('.cmdp-search__input')!
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
  }

  it('shows one primary usage destination for English and Chinese queries', async () => {
    i18n.global.locale.value = 'en'
    const el = await mountPalette()

    await search(el, 'usage')
    expect(Array.from(el.querySelectorAll('.cmdp-option__label')).map(node => node.textContent))
      .toEqual(['View usage'])

    await search(el, '用量')
    expect(Array.from(el.querySelectorAll('.cmdp-option__label')).map(node => node.textContent))
      .toEqual(['View usage'])
    expect(el.querySelector('.cmdp-group-label')?.textContent).toBe('Work')
  })

  // Gap4: the mode command kind must use the canonical hyphen form so the
  // active marker lights for OmniTranscend (core broadcasts `omni-transcend`).
  it('marks the Omni Transcend command active for the canonical hyphen kind', async () => {
    i18n.global.locale.value = 'en'
    useActiveModeStore().mode = { kind: 'omni-transcend', mode: 'omni-transcend' }
    const el = await mountPalette()

    await search(el, 'omni')
    const options = Array.from(el.querySelectorAll('.cmdp-option'))
    const omni = options.find(option =>
      option.querySelector('.cmdp-option__label')?.textContent?.includes('Omni'),
    )
    expect(omni, 'Omni Transcend command should be listed').toBeTruthy()
    expect(
      omni!.querySelector('.cmdp-option__sub')?.textContent?.trim(),
      'active marker must light for the hyphen kind',
    ).toBeTruthy()
  })
})
