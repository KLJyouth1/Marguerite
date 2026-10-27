// @vitest-environment happy-dom

import { createApp, nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import i18n from '@/i18n'
import type { ChatRenderedMessage } from '@/types/chat'
import SubagentCard from './SubagentCard.vue'

const mounted: Array<ReturnType<typeof createApp>> = []

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

function mount(onFocus = vi.fn()) {
  i18n.global.locale.value = 'en'
  const message = {
    text: JSON.stringify({
      child_session_key: 'agent:main:subagent:abcd1234',
      status: 'completed',
      result: { text: 'finished' },
    }),
    displayRole: 'subagent',
    ts: '2026-10-26T10:00:00Z',
  } as ChatRenderedMessage
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp(SubagentCard, { message, onFocus })
  app.use(i18n)
  app.mount(root)
  mounted.push(app)
  return { root, onFocus }
}

describe('SubagentCard focus affordance', () => {
  it('emits the child session key when the card is clicked', async () => {
    const { root, onFocus } = mount()
    await nextTick()
    root.querySelector<HTMLElement>('.agent-card')!.click()
    expect(onFocus).toHaveBeenCalledWith({ key: 'agent:main:subagent:abcd1234' })
  })

  it('exposes an explicit button for keyboard and screen-reader users', async () => {
    const { root, onFocus } = mount()
    await nextTick()
    const button = root.querySelector<HTMLButtonElement>('.agent-card__focus')
    expect(button?.getAttribute('aria-label')).toContain('Subagents')
    button!.click()
    expect(onFocus).toHaveBeenCalledTimes(1)
    // JSON 折叠属于卡片内部交互，不应顺带打开右栏（点击冒泡被阻止）。
    root.querySelector<HTMLElement>('details summary')!.click()
    expect(onFocus).toHaveBeenCalledTimes(1)
  })
})
