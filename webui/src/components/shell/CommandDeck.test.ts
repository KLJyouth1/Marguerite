// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import CommandDeck from './CommandDeck.vue'
import type { WorkPhase } from '@/composables/shell/useWorkPhase'

const mounted: App[] = []

const EXECUTE: WorkPhase = { kind: 'execute', reason: 'toolsRunning', count: 2 }

function stubMatchMedia(reduced: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: reduced && query.includes('prefers-reduced-motion'),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }))
}

async function mountDeck(
  actions: Array<{ id: string; label: string; hint?: string }>,
  phase: WorkPhase = EXECUTE,
) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const onActivate = vi.fn()
  const app = createApp(CommandDeck, { phase, actions, onActivate })
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  await nextTick()
  return { el, onActivate }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  stubMatchMedia(false)
  document.body.innerHTML = ''
})

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
  i18n.global.locale.value = 'en'
})

describe('CommandDeck', () => {
  it('renders nothing when the parent supplies no actions', async () => {
    const { el } = await mountDeck([])
    expect(el.querySelector('[data-testid="command-deck"]')).toBeNull()
  })

  it('shows the phase, its honest reason, and every parent-supplied action', async () => {
    const { el } = await mountDeck([
      { id: 'stop-turn', label: 'Stop this turn' },
      { id: 'open-settings', label: 'Open settings', hint: 'Ctrl+,' },
    ])

    const deck = el.querySelector('[data-testid="command-deck"]')
    expect(deck?.getAttribute('data-phase')).toBe('execute')
    expect(el.querySelector('[data-testid="command-deck-phase"]')?.textContent).toContain('Executing')
    expect(el.querySelector('[data-testid="command-deck-reason"]')?.textContent).toContain('2')
    expect(el.querySelectorAll('.cmd-deck__action')).toHaveLength(2)
    expect(el.querySelector('[data-action="open-settings"]')?.textContent).toContain('Ctrl+,')
  })

  it('marks the deck as reduced-motion when the system asks for it', async () => {
    stubMatchMedia(true)
    const { el } = await mountDeck([{ id: 'open-settings', label: 'Open settings' }])
    expect(el.querySelector('[data-testid="command-deck"]')?.classList.contains('cmd-deck--reduced'))
      .toBe(true)
  })

  it('activates an action with Enter and Space, and advertises those keys', async () => {
    const { el, onActivate } = await mountDeck([{ id: 'open-settings', label: 'Open settings' }])
    const button = el.querySelector<HTMLButtonElement>('[data-action="open-settings"]')!

    expect(button.getAttribute('aria-keyshortcuts')).toBe('Enter Space')

    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
    await nextTick()
    expect(onActivate).toHaveBeenCalledTimes(1)
    expect(onActivate).toHaveBeenCalledWith('open-settings')

    button.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }))
    await nextTick()
    expect(onActivate).toHaveBeenCalledTimes(2)
    expect(onActivate).toHaveBeenLastCalledWith('open-settings')
  })

  it('keeps pointer activation working alongside the keyboard path', async () => {
    const { el, onActivate } = await mountDeck([{ id: 'open-settings', label: 'Open settings' }])
    el.querySelector<HTMLButtonElement>('[data-action="open-settings"]')!.click()
    await nextTick()
    expect(onActivate).toHaveBeenCalledTimes(1)
    expect(onActivate).toHaveBeenCalledWith('open-settings')
  })
})
