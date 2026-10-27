// @vitest-environment happy-dom

import { createApp, h, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import i18n from '@/i18n'
import type { ChatRenderedMessage } from '@/types/chat'
import AssistantMessage from './AssistantMessage.vue'

const apps: App[] = []

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

function message(text: string): ChatRenderedMessage {
  return {
    role: 'assistant',
    displayRole: 'assistant',
    roleLabel: 'Assistant',
    text,
    timeStr: '',
    showHeader: false,
    parts: [{ type: 'text', key: 'answer', html: `<p>${text}</p>`, rawText: text }],
  } as ChatRenderedMessage
}

function mount(text: string, onShowFullMessage = vi.fn()) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp({
    render: () => h(AssistantMessage, {
      message: message(text),
      index: 0,
      sessionKey: 'session-a',
      shareMode: false,
      shareSelected: false,
      shareMessageId: 'assistant-1',
      renderMarkdown: (value: string) => `<p>${value}</p>`,
      fmtTok: (value: number) => String(value),
      toolCallGroups: () => [],
      isToolGroupOpen: () => false,
      isToolItemOpen: () => false,
      toolGroupStatusText: () => '',
      toolStatusText: () => '',
      toolSecondaryText: () => '',
      copyMessage: async () => true,
      onShowFullMessage,
    }),
  })
  apps.push(app)
  app.use(i18n)
  app.mount(el)
  return { el, onShowFullMessage }
}

describe('AssistantMessage full-message affordance', () => {
  it('offers the full message only when the answer exceeds the reading threshold', async () => {
    const long = await mount('x'.repeat(640))
    await nextTick()
    expect(long.el.querySelector('[data-testid="full-message-action"]')).toBeTruthy()

    const short = await mount('short answer')
    await nextTick()
    expect(short.el.querySelector('[data-testid="full-message-action"]')).toBeNull()
  })

  it('emits the message instead of copying it', async () => {
    const { el, onShowFullMessage } = mount('y'.repeat(640))
    await nextTick()
    el.querySelector<HTMLButtonElement>('[data-testid="full-message-action"]')!.click()
    expect(onShowFullMessage).toHaveBeenCalledTimes(1)
    expect(onShowFullMessage.mock.calls[0]?.[0]?.text).toHaveLength(640)
  })
})
