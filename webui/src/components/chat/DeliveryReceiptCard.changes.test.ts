// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import DeliveryReceiptCard from './DeliveryReceiptCard.vue'

const rpc = vi.hoisted(() => ({
  isConnected: true,
  supportsMethod: vi.fn(),
  call: vi.fn(),
  waitForConnection: vi.fn(async () => true),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => rpc,
}))

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

beforeEach(() => {
  rpc.isConnected = true
  rpc.supportsMethod.mockReset()
  rpc.call.mockReset()
  rpc.waitForConnection.mockReset()
  rpc.waitForConnection.mockResolvedValue(true)
})

async function settle() {
  await nextTick()
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

describe('DeliveryReceiptCard 文件列表', () => {
  it('点开文件行后加载并渲染内联 diff', async () => {
    const diff = ['--- a/src/a.ts', '+++ b/src/a.ts', '@@ -1 +1,2 @@', '-old', '+new', '+extra'].join('\n')
    rpc.supportsMethod.mockImplementation((method: string) =>
      method === 'task.receipt' || method === 'sessions.fileChanges',
    )
    rpc.call.mockImplementation(async (method: string) => {
      if (method === 'task.receipt') {
        return {
          ok: true,
          counts: { changes: 1, artifacts: 0, toolCalls: 1 },
          changes: [{ path: 'src/a.ts', kind: 'modify', tool: 'edit_file' }],
          verification: { status: 'unverified', checks: [] },
          budget: {},
          unattested: [],
        }
      }
      if (method === 'sessions.fileChanges') {
        return {
          ok: true,
          changes: [{
            path: 'src/a.ts',
            kind: 'modify',
            tool: 'edit_file',
            added: 2,
            removed: 1,
            diff,
          }],
        }
      }
      return {}
    })

    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp(DeliveryReceiptCard, {
      sessionKey: 's1',
      since: '2026-10-25T10:00:00Z',
      until: '2026-10-25T10:00:05Z',
    })
    app.use(i18n)
    app.mount(host)
    apps.push(app)
    await settle()

    host.querySelector<HTMLButtonElement>('.receipt__head')?.click()
    await settle()

    const row = host.querySelector<HTMLButtonElement>('[data-testid="receipt-change-row"]')
    expect(row).toBeTruthy()
    expect(row?.getAttribute('aria-expanded')).toBe('false')
    row?.click()
    await settle()

    expect(row?.getAttribute('aria-expanded')).toBe('true')
    const diffBox = host.querySelector('[data-testid="file-change-diff"]')
    expect(diffBox).toBeTruthy()
    expect(diffBox?.querySelectorAll('.fcv__line--add')).toHaveLength(2)
    expect(diffBox?.querySelectorAll('.fcv__line--del')).toHaveLength(1)
  })
})
