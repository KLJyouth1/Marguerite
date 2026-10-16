// @vitest-environment happy-dom

import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick } from 'vue'
import i18n from '@/i18n'
import type { ChatRenderedMessage } from '@/types/chat'
import UserMessage from './UserMessage.vue'

async function renderText(text: string) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const message: ChatRenderedMessage = {
    id: 'long-text',
    role: 'user',
    displayRole: 'user',
    roleLabel: 'You',
    text,
    timeStr: '',
    showHeader: false,
  }
  const app = createApp(UserMessage, {
    message,
    shareMode: false,
    shareSelected: false,
    shareMessageId: message.id,
    stripTimePrefix: (value: string) => value,
    copyMessage: async () => true,
    downloadAttachment: async () => true,
  })
  app.use(i18n)
  app.mount(host)
  await nextTick()
  return { app, host }
}

afterEach(() => {
  document.body.innerHTML = ''
  i18n.global.locale.value = 'en'
})

// Guards against the transcript blow-out: a huge paste used to render one
// unbounded bubble with no way to collapse it.
describe('UserMessage long paste', () => {
  it('clamps a huge paste and offers an expand toggle', async () => {
    const { app, host } = await renderText('x'.repeat(5000))

    const bubble = host.querySelector('.msg-user-bubble')
    expect(bubble?.classList.contains('msg-user-bubble--clamped')).toBe(true)

    const toggle = host.querySelector('.msg-user-expand') as HTMLButtonElement | null
    expect(toggle).toBeTruthy()
    expect(toggle?.getAttribute('aria-expanded')).toBe('false')
    app.unmount()
  })

  it('expands on toggle and drops the clamp', async () => {
    const { app, host } = await renderText('x'.repeat(5000))

    const toggle = host.querySelector('.msg-user-expand') as HTMLButtonElement
    toggle.click()
    await nextTick()

    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    expect(
      host.querySelector('.msg-user-bubble')?.classList.contains('msg-user-bubble--clamped'),
    ).toBe(false)
    app.unmount()
  })

  it('clamps a short-but-many-line paste', async () => {
    const lines = Array.from({ length: 40 }, (_, i) => `line ${i}`).join('\n')
    const { app, host } = await renderText(lines)

    expect(host.querySelector('.msg-user-bubble--clamped')).toBeTruthy()
    app.unmount()
  })

  it('leaves a short message untouched', async () => {
    const { app, host } = await renderText('short message')

    expect(host.querySelector('.msg-user-expand')).toBeNull()
    expect(host.querySelector('.msg-user-bubble--clamped')).toBeNull()
    app.unmount()
  })
})
