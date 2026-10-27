// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import ChatContextPressureChip from './ChatContextPressureChip.vue'

const mounted: App[] = []

function mount(warning: { pct: number; usedK: number; windowK: number } | null) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(ChatContextPressureChip, {
    warning,
    onCompact: () => { clicks += 1 },
  })
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  return el
}

let clicks = 0

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
  document.body.innerHTML = ''
  i18n.global.locale.value = 'en'
  clicks = 0
})

describe('ChatContextPressureChip', () => {
  it('renders nothing when the gateway reports no pressure', async () => {
    const el = mount(null)
    await nextTick()
    expect(el.querySelector('[data-testid="context-pressure-chip"]')).toBeNull()
  })

  it('shows the percentage once the gateway crosses its warning ratio', async () => {
    const el = mount({ pct: 88, usedK: 88, windowK: 100 })
    await nextTick()

    const chip = el.querySelector<HTMLButtonElement>('[data-testid="context-pressure-chip"]')
    expect(chip).not.toBeNull()
    expect(chip?.textContent).toContain('88')
    expect(chip?.getAttribute('title')).toContain('88')
    expect(chip?.classList.contains('ctx-pressure--critical')).toBe(false)
  })

  it('switches to the critical tone near saturation', async () => {
    const el = mount({ pct: 97, usedK: 97, windowK: 100 })
    await nextTick()
    expect(el.querySelector('[data-testid="context-pressure-chip"]')
      ?.classList.contains('ctx-pressure--critical')).toBe(true)
  })

  it('asks the chat view to compact when clicked', async () => {
    const el = mount({ pct: 90, usedK: 90, windowK: 100 })
    await nextTick()
    el.querySelector<HTMLButtonElement>('[data-testid="context-pressure-chip"]')?.click()
    expect(clicks).toBe(1)
  })
})
