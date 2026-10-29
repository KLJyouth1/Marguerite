// @vitest-environment happy-dom
// F-09 等待指示器：长等待时显示相位 + 秒数脉冲；工具/审批/融合挂起态显式标注；
// 不可见时不渲染（避免与运行中 UI 重复）。
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import HeartbeatWaitIndicator from '@/components/chat/HeartbeatWaitIndicator.vue'

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  i18n.global.locale.value = 'zh-Hans'
})

async function mount(props: Record<string, unknown>) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(HeartbeatWaitIndicator, props)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return el
}

describe('HeartbeatWaitIndicator (F-09)', () => {
  it('is hidden when not waiting', async () => {
    const el = await mount({ visible: false, seconds: 12 })
    expect(el.querySelector('[data-testid="chat-wait-indicator"]')).toBeNull()
  })

  it('shows the phase label and ticking seconds while waiting', async () => {
    const el = await mount({ visible: true, phaseLabel: '等待模型响应', seconds: 63 })
    const node = el.querySelector('[data-testid="chat-wait-indicator"]')!
    expect(node.textContent).toContain('等待模型响应')
    expect(node.textContent).toContain('63s')
    expect(node.getAttribute('data-suspended')).toBe('false')
  })

  it('labels the suspended state from the watchdog reason (consistency)', async () => {
    const el = await mount({
      visible: true,
      suspended: true,
      suspendLabel: '等待工具执行完成',
      seconds: 104,
    })
    const node = el.querySelector('[data-testid="chat-wait-indicator"]')!
    expect(node.getAttribute('data-suspended')).toBe('true')
    expect(node.textContent).toContain('等待工具执行完成')
  })

  it('can hide the seconds (pulse-only mode)', async () => {
    const el = await mount({ visible: true, phaseLabel: '仍在运行', seconds: 30, showSeconds: false })
    const node = el.querySelector('[data-testid="chat-wait-indicator"]')!
    expect(node.textContent).toContain('仍在运行')
    expect(node.textContent).not.toContain('30s')
  })

  // 真机审计 2026-10-29 P1-3：并行会话里第二页干等 27s，屏幕上只有一个转圈，
  // 用户分不清"卡死"和"在排队"。补的这行说明必须渲染出来，但**不能**塞进
  // 那个 aria-live 的句子 —— 否则读屏每秒重播一次（秒数每秒都在变）。
  it('renders the hint as a separate note, outside the aria-live status line', async () => {
    const el = await mount({
      visible: true,
      phaseLabel: '等待模型',
      seconds: 27,
      hint: '已等 27s · 模型调用为串行（默认 Agent 模式），切换 Bloom 可并行',
    })
    const status = el.querySelector('[data-testid="chat-wait-indicator"]')!
    const hint = el.querySelector('[data-testid="chat-wait-indicator-hint"]')!
    expect(hint.textContent).toContain('切换 Bloom 可并行')
    expect(hint.getAttribute('role')).toBe('note')
    // 关键断言：live region 里不含这句长文案
    expect(status.textContent).not.toContain('切换 Bloom')
  })

  it('renders no hint node when hint is empty', async () => {
    const el = await mount({ visible: true, phaseLabel: '等待模型', seconds: 5 })
    expect(el.querySelector('[data-testid="chat-wait-indicator-hint"]')).toBeNull()
  })
})