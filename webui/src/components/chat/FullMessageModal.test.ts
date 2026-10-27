// @vitest-environment happy-dom

import { createApp, defineComponent, h, nextTick, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import i18n from '@/i18n'
import { copyTextWithFallback } from '@/utils/browser'
import FullMessageModal from './FullMessageModal.vue'

vi.mock('@/utils/browser', () => ({
  copyTextWithFallback: vi.fn().mockResolvedValue(undefined),
}))

let app: ReturnType<typeof createApp> | null = null

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
  vi.mocked(copyTextWithFallback).mockClear()
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

function mount(content: string, open = true) {
  const root = document.createElement('div')
  document.body.appendChild(root)
  const isOpen = ref(open)
  const Host = defineComponent({
    setup() {
      return () => h(FullMessageModal, {
        open: isOpen.value,
        content,
        onClose: () => { isOpen.value = false },
      })
    },
  })
  app = createApp(Host)
  app.use(i18n)
  app.mount(root)
  return { root, isOpen }
}

describe('FullMessageModal', () => {
  it('renders the whole message as sanitized markdown with a character count', async () => {
    const content = '# Heading\n\nA full answer that was previously clipped.'
    const { root } = mount(content)
    await nextTick()

    const scroll = root.querySelector('.full-message__scroll')
    expect(scroll?.textContent).toContain('Heading')
    expect(scroll?.querySelector('p')?.textContent).toContain('A full answer that was previously clipped.')
    expect(root.querySelector('.full-message__meta')?.textContent).toContain(`${content.length} characters`)
    expect(document.querySelector('[role="dialog"]')).toBeTruthy()
  })

  it('copies the raw message and confirms without altering it', async () => {
    const content = '**bold** text'
    const { root } = mount(content)
    await nextTick()
    root.querySelector<HTMLButtonElement>('button[aria-label="Copy message"]')!.click()
    await new Promise(resolve => setTimeout(resolve, 0))
    await nextTick()
    expect(copyTextWithFallback).toHaveBeenCalledWith(content)
    expect(root.querySelector('button[aria-label="Copied"]')).toBeTruthy()
  })

  it('renders nothing while closed', async () => {
    const { root } = mount('hidden', false)
    await nextTick()
    expect(root.querySelector('[role="dialog"]')).toBeNull()
  })
})
