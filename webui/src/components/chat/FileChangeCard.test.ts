// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import { createI18n } from 'vue-i18n'

import FileChangeCard from './FileChangeCard.vue'
import type { FileChangePayload } from '@/types/rpc'

const messages = {
  chat: {
    fileChange: {
      title: 'File changes',
      create: 'Create',
      modify: 'Modify',
      delete: 'Delete',
      toggle: 'Expand/collapse {path}',
      noContent: 'No content diff',
      unchanged: 'Unchanged',
      accept: 'Accept',
      accepted: 'Accepted',
      reject: 'Reject',
      empty: 'No file changes yet',
      loadFailed: 'Failed to load file changes',
    },
  },
}

const mounted: Array<{ app: App; el: HTMLElement }> = []

async function mountCard(change: FileChangePayload) {
  const handlers = {
    accept: vi.fn(),
    reject: vi.fn(),
  }
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(FileChangeCard, {
    change,
    onAccept: handlers.accept,
    onReject: handlers.reject,
  })
  app.use(createI18n({ legacy: false, locale: 'en', messages: { en: messages } }))
  app.mount(el)
  mounted.push({ app, el })
  await nextTick()
  return { el, handlers }
}

afterEach(() => {
  while (mounted.length) {
    const { app, el } = mounted.pop()!
    app.unmount()
    el.remove()
  }
  vi.restoreAllMocks()
})

function change(overrides: Partial<FileChangePayload> = {}): FileChangePayload {
  return {
    path: 'src/main.rs',
    kind: 'modify',
    before: 'a\nb',
    after: 'a\nB',
    tool: 'edit_file',
    ts: 1,
    ...overrides,
  }
}

describe('FileChangeCard', () => {
  it('renders the kind badge and add/remove summary', async () => {
    const { el } = await mountCard(change())
    expect(el.querySelector('.file-change-card__badge')?.textContent).toBe('Modify')
    expect(el.querySelector('.file-change-card__meta')?.textContent).toBe('+1 -1')
  })

  it('reveals diff lines only after expanding', async () => {
    const { el } = await mountCard(change())
    expect(el.querySelector('.file-change-card__diff')).toBeNull()

    const header = el.querySelector<HTMLButtonElement>('.file-change-card__header')!
    header.click()
    await nextTick()

    const lines = el.querySelectorAll<HTMLElement>('.file-change-card__line')
    expect(lines.length).toBe(3)
    expect(lines[0].dataset.type).toBe('ctx')
    expect(lines[1].dataset.type).toBe('del')
    expect(lines[2].dataset.type).toBe('add')
  })

  it('labels create and delete kinds distinctly', async () => {
    const created = await mountCard(change({ kind: 'create', before: '', after: 'fn main(){}' }))
    expect(created.el.querySelector('.file-change-card__badge')?.textContent).toBe('Create')

    const deleted = await mountCard(change({ kind: 'delete', before: 'gone', after: '' }))
    expect(deleted.el.querySelector('.file-change-card__badge')?.textContent).toBe('Delete')
  })

  it('emits accept once and flips the accept label', async () => {
    const { el, handlers } = await mountCard(change())
    const accept = el.querySelector<HTMLButtonElement>('.file-change-card__action--accept')!
    accept.click()
    await nextTick()

    expect(handlers.accept).toHaveBeenCalledTimes(1)
    expect(accept.textContent).toContain('Accepted')
    expect(accept.disabled).toBe(true)
  })

  it('emits reject with the change payload', async () => {
    const payload = change()
    const { el, handlers } = await mountCard(payload)
    const reject = el.querySelector<HTMLButtonElement>('.file-change-card__action--reject')!
    reject.click()

    expect(handlers.reject).toHaveBeenCalledWith(payload)
  })
})
