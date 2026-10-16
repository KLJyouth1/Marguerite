// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import i18n from '@/i18n'
import type { ChatRenderedMessage } from '@/types/chat'
import { AUTH_FAILED } from '@/utils/chat/errors'
import SystemMessage from './SystemMessage.vue'

function errorMessage(overrides: Partial<ChatRenderedMessage> = {}): ChatRenderedMessage {
  return {
    role: 'error',
    displayRole: 'error',
    roleLabel: 'Error',
    text: 'Automatic execution paused after repeated sandbox denials.',
    timeStr: '',
    ts: null,
    showHeader: true,
    ...overrides,
  }
}

async function mountMsg(
  message: ChatRenderedMessage,
  onResume?: () => void,
  onRetry?: () => void,
) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(SystemMessage, {
    message,
    onResume,
    onRetry,
  })
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return { app, el }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

describe('SystemMessage sandbox resume', () => {
  it('renders a Resume button for a sandbox-pause error and emits resume once on click', async () => {
    const onResume = vi.fn()
    const { app, el } = await mountMsg(
      errorMessage({ errorCode: 'sandbox_threshold_exceeded' }),
      onResume,
    )
    const btn = el.querySelector<HTMLButtonElement>('.msg-error-card__resume')
    expect(btn).not.toBeNull()
    expect(btn?.textContent).toContain('Resume execution')

    btn?.click()
    await nextTick()
    expect(onResume).toHaveBeenCalledTimes(1)
    // Disabled after one click so a repeated click cannot fire duplicate resumes.
    expect(btn?.disabled).toBe(true)
    btn?.click()
    await nextTick()
    expect(onResume).toHaveBeenCalledTimes(1)
    app.unmount()
  })

  it('does not render a Resume button for other terminal error codes', async () => {
    const { app, el } = await mountMsg(errorMessage({ errorCode: 'iteration_timeout' }))
    expect(el.querySelector('.msg-error-card__resume')).toBeNull()
    app.unmount()
  })

  it('does not render a Resume button when the error carries no code', async () => {
    const { app, el } = await mountMsg(errorMessage())
    expect(el.querySelector('.msg-error-card__resume')).toBeNull()
    app.unmount()
  })

  it('does not render a Resume button on a non-error system message', async () => {
    const { app, el } = await mountMsg(
      errorMessage({ role: 'system', displayRole: 'system', errorCode: 'sandbox_threshold_exceeded' }),
    )
    expect(el.querySelector('.msg-error-card__resume')).toBeNull()
    app.unmount()
  })
})

describe('SystemMessage retry escape hatch', () => {
  it('offers a Retry action for an ordinary failed turn and emits retry once on click', async () => {
    // 真机实测：计划子步骤失败 / 超时这类失败此前没有任何按钮 = 死路。
    const onRetry = vi.fn()
    const { app, el } = await mountMsg(
      errorMessage({ errorCode: 'iteration_timeout' }),
      undefined,
      onRetry,
    )
    const retry = el.querySelector<HTMLButtonElement>('.msg-error-card__action')
    expect(retry, '普通失败轮次必须给一个出口').not.toBeNull()
    expect(retry?.textContent).toContain('Retry')
    retry?.click()
    await nextTick()
    expect(onRetry).toHaveBeenCalledTimes(1)
    app.unmount()
  })

  it('keeps the sandbox-pause card on Resume only (retry is a different recovery)', async () => {
    const { app, el } = await mountMsg(errorMessage({ errorCode: 'sandbox_threshold_exceeded' }))
    expect(el.querySelector('.msg-error-card__resume')).not.toBeNull()
    expect(el.querySelector('.msg-error-card__action')).toBeNull()
    app.unmount()
  })

  it('offers both settings and retry for an auth failure', async () => {
    const { app, el } = await mountMsg(errorMessage({ errorCode: AUTH_FAILED }))
    const labels = Array.from(el.querySelectorAll('.msg-error-card__action')).map(
      (n) => n.textContent?.trim() ?? '',
    )
    expect(labels).toHaveLength(2)
    expect(labels.some((l) => l.includes('Retry'))).toBe(true)
    app.unmount()
  })
})
