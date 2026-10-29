// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import ChatContextMeter from './ChatContextMeter.vue'

const mounted: App[] = []

function mount(status: unknown) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(ChatContextMeter, { status })
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  return el
}

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
  document.body.innerHTML = ''
  i18n.global.locale.value = 'en'
})

describe('ChatContextMeter', () => {
  it('renders nothing when the gateway sent no contextStatus', async () => {
    // 关键否定面：纯 Rust 且尚未上报的会话 = 没有这块数据。
    // 渲染一个 0% 会教用户误读成「窗口很空」。
    const el = mount(null)
    await nextTick()
    expect(el.querySelector('[data-testid="context-meter"]')).toBeNull()
  })

  it('renders nothing when the window is unknown', async () => {
    const el = mount({ contextTokens: 1200, contextWindowTokens: 0, tier: 'normal' })
    await nextTick()
    expect(el.querySelector('[data-testid="context-meter"]')).toBeNull()
  })

  it('shows used / window / per-band breakdown from the real numbers', async () => {
    const el = mount({
      contextTokens: 60_000,
      contextWindowTokens: 100_000,
      chatTokens: 20_000,
      toolTokens: 40_000,
      pressure: 0.6,
      tier: 'advise',
      ladder: [0.6, 0.75, 0.85],
    })
    await nextTick()

    const meter = el.querySelector('[data-testid="context-meter"]')
    expect(meter).not.toBeNull()
    expect(meter?.classList.contains('ctx-meter--advise')).toBe(true)
    expect(el.querySelector('[data-testid="context-meter-used"]')?.textContent).toContain('60')
    expect(meter?.textContent).toContain('100')
    expect(el.querySelector('[data-testid="context-meter-tool"]')?.textContent).toContain('40')
    expect(meter?.textContent).toContain('60%')
    expect(el.querySelector('[data-testid="context-meter-window-approx"]')).toBeNull()
  })

  it('derives the tier from the ladder when the gateway omits it', async () => {
    const el = mount({
      contextTokens: 90_000,
      contextWindowTokens: 100_000,
      pressure: 0.9,
      ladder: [0.6, 0.75, 0.85],
    })
    await nextTick()
    expect(el.querySelector('[data-testid="context-meter"]')
      ?.classList.contains('ctx-meter--compact')).toBe(true)
  })

  it('labels an approximate window instead of presenting it as the real one', async () => {
    const el = mount({
      contextTokens: 500,
      contextWindowTokens: 4000,
      pressure: 0.125,
      tier: 'normal',
      windowSource: 'budget_fallback',
    })
    await nextTick()
    expect(el.querySelector('[data-testid="context-meter-window-approx"]')).not.toBeNull()
  })
})
