// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import i18n from '@/i18n'

const mockRpc = vi.hoisted(() => ({
  isConnected: true,
  waitForConnection: vi.fn(async () => true),
  call: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => mockRpc,
}))

import HandoffNoticeCard from './HandoffNoticeCard.vue'

async function mountCard(props: { basePath: string }) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(HandoffNoticeCard, { ...props })
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return {
    el,
    unmount: () => {
      app.unmount()
      el.remove()
    },
  }
}

async function settle() {
  // 让 waitForConnection 与 rpc.call 的 microtask 链排空
  for (let i = 0; i < 5; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

describe('HandoffNoticeCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    i18n.global.locale.value = 'en'
    document.body.innerHTML = ''
  })

  it('renders nothing when the workspace has no handoff directory', async () => {
    mockRpc.call.mockRejectedValue(new Error('PATH_NOT_FOUND'))
    const { el, unmount } = await mountCard({ basePath: 'D:\\ws' })
    await settle()
    expect(el.querySelector('[data-testid="handoff-notice"]')).toBeNull()
    unmount()
  })

  it('shows the newest markdown handoff and emits use/open', async () => {
    mockRpc.call.mockResolvedValue({
      entries: [
        { name: '2026-10-26T10-00-00.md', path: 'D:\\ws\\handoff\\2026-10-26T10-00-00.md', kind: 'file', selectable: true },
        { name: '2026-10-27T08-00-00.md', path: 'D:\\ws\\handoff\\2026-10-27T08-00-00.md', kind: 'file', selectable: true },
        { name: 'notes.txt', path: 'D:\\ws\\handoff\\notes.txt', kind: 'file', selectable: true },
        { name: 'sub', path: 'D:\\ws\\handoff\\sub', kind: 'directory', selectable: false },
      ],
    })
    const onUse = vi.fn()
    const onOpen = vi.fn()
    const el = document.createElement('div')
    document.body.appendChild(el)
    const app = createApp(HandoffNoticeCard, {
      basePath: 'D:\\ws',
      onUse: (name: string) => onUse(name),
      onOpen: (p: string) => onOpen(p),
    })
    app.use(i18n)
    app.mount(el)
    await settle()

    expect(el.querySelector('[data-testid="handoff-notice"]')).not.toBeNull()
    expect(el.textContent).toContain('2026-10-27T08-00-00.md')

    ;(el.querySelector('[data-testid="handoff-notice-use"]') as HTMLButtonElement).click()
    await nextTick()
    expect(onUse).toHaveBeenCalledWith('2026-10-27T08-00-00.md')

    ;(el.querySelector('[data-testid="handoff-notice-open"]') as HTMLButtonElement).click()
    await nextTick()
    expect(onOpen).toHaveBeenCalledWith('D:\\ws\\handoff\\2026-10-27T08-00-00.md')
    app.unmount()
    el.remove()
  })

  it('queries the handoff subdirectory of the given base path', async () => {
    mockRpc.call.mockResolvedValue({ entries: [] })
    const { unmount } = await mountCard({ basePath: 'D:\\ws\\proj\\' })
    await settle()
    expect(mockRpc.call).toHaveBeenCalledWith('sandbox.path.list', { path: 'D:\\ws\\proj/handoff' })
    unmount()
  })

  it('does not call RPC when basePath is empty (workspace unbound)', async () => {
    const { unmount } = await mountCard({ basePath: '' })
    await settle()
    expect(mockRpc.call).not.toHaveBeenCalled()
    expect(document.querySelector('[data-testid="handoff-notice"]')).toBeNull()
    unmount()
  })
})
