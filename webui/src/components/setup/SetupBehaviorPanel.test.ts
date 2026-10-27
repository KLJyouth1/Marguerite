// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import SetupBehaviorPanel from './SetupBehaviorPanel.vue'

const mounted: App[] = []
const KEY = 'marguerite.notify.turnEnded'

function mount() {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(SetupBehaviorPanel, {
    panel: { autoSessionTitles: true, autoSessionTitlesDirty: false, statusText: 'ok' },
  })
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  return el
}

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
  document.body.innerHTML = ''
  localStorage.removeItem(KEY)
})

describe('SetupBehaviorPanel turn notification toggle', () => {
  it('reflects the persisted preference (default on)', async () => {
    const el = mount()
    await nextTick()
    const input = el.querySelector<HTMLInputElement>('input[name="turn_end_notify"]')
    expect(input).not.toBeNull()
    expect(input?.checked).toBe(true)
  })

  it('honours an explicitly disabled preference', async () => {
    localStorage.setItem(KEY, '0')
    const el = mount()
    await nextTick()
    expect(el.querySelector<HTMLInputElement>('input[name="turn_end_notify"]')?.checked).toBe(false)
  })

  it('writes the preference immediately when switched', async () => {
    const el = mount()
    await nextTick()
    const input = el.querySelector<HTMLInputElement>('input[name="turn_end_notify"]')!
    input.checked = false
    input.dispatchEvent(new Event('change'))
    await nextTick()
    expect(localStorage.getItem(KEY)).toBe('0')
  })
})
