// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import i18n from '@/i18n'
import zhHans from '@/locales/zh-Hans.json'
import TurnSuggestions from './TurnSuggestions.vue'

async function mountSuggestions(props: { canRevert?: boolean } = {}) {
  const onView = vi.fn()
  const onRunTests = vi.fn()
  const onRevert = vi.fn()
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(TurnSuggestions, { ...props, onView, onRunTests, onRevert })
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return { app, el, onView, onRunTests, onRevert }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

describe('TurnSuggestions', () => {
  it('renders the three next-step chips and forwards each intent', async () => {
    const { app, el, onView, onRunTests, onRevert } = await mountSuggestions({ canRevert: true })

    const view = el.querySelector<HTMLButtonElement>('[data-testid="turn-suggestion-view"]')
    const tests = el.querySelector<HTMLButtonElement>('[data-testid="turn-suggestion-tests"]')
    const revert = el.querySelector<HTMLButtonElement>('[data-testid="turn-suggestion-revert"]')
    expect(view?.textContent).toContain('View diff')
    expect(tests?.textContent).toContain('Run tests')
    expect(revert?.textContent).toContain('Revert this turn')

    view?.click()
    tests?.click()
    revert?.click()
    await nextTick()

    expect(onView).toHaveBeenCalledTimes(1)
    expect(onRunTests).toHaveBeenCalledTimes(1)
    expect(onRevert).toHaveBeenCalledTimes(1)
    app.unmount()
  })

  it('drops the revert chip when this deployment cannot revert', async () => {
    const { app, el } = await mountSuggestions({ canRevert: false })

    expect(el.querySelector('[data-testid="turn-suggestion-view"]')).not.toBeNull()
    expect(el.querySelector('[data-testid="turn-suggestion-tests"]')).not.toBeNull()
    expect(el.querySelector('[data-testid="turn-suggestion-revert"]')).toBeNull()
    app.unmount()
  })

  it('renders localized labels', async () => {
    i18n.global.setLocaleMessage('zh-Hans', zhHans)
    i18n.global.locale.value = 'zh-Hans'
    const { app, el } = await mountSuggestions({ canRevert: true })

    const labels = [...el.querySelectorAll<HTMLButtonElement>('.turn-suggestions__chip')]
      .map(button => button.textContent?.trim())
    expect(labels).toEqual(['查看 diff', '运行测试', '撤销本次改动'])
    app.unmount()
  })
})
