// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import FileChangeDiffViewer from './FileChangeDiffViewer.vue'

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

async function mountViewer(props: Record<string, unknown>): Promise<HTMLElement> {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(FileChangeDiffViewer, props)
  app.use(i18n)
  app.mount(host)
  apps.push(app)
  await settle()
  return host
}

const DIFF = [
  '--- a/src/a.ts',
  '+++ b/src/a.ts',
  '@@ -1,2 +1,3 @@',
  ' keep',
  '-old',
  '+new',
  '+extra',
].join('\n')

describe('FileChangeDiffViewer', () => {
  it('渲染 +N/-M、行级颜色类与黄色 gutter', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'sessions.fileChanges')
    rpc.call.mockResolvedValue({
      ok: true,
      changes: [{
        path: 'src/a.ts',
        kind: 'modify',
        tool: 'edit_file',
        added: 2,
        removed: 1,
        diff: DIFF,
      }],
    })
    const host = await mountViewer({ sessionKey: 's1', initialOpenPath: 'src/a.ts' })

    expect(host.querySelector('[data-testid="file-change-row"]')).toBeTruthy()
    expect(host.querySelector('.fcv__added')?.textContent).toBe('+2')
    expect(host.querySelector('.fcv__removed')?.textContent).toBe('-1')
    const diff = host.querySelector('[data-testid="file-change-diff"]')
    expect(diff).toBeTruthy()
    expect(diff?.querySelectorAll('.fcv__line--add')).toHaveLength(2)
    expect(diff?.querySelectorAll('.fcv__line--del')).toHaveLength(1)
    expect(diff?.querySelectorAll('.fcv__line--hunk')).toHaveLength(1)
    expect(diff?.querySelectorAll('.fcv__gutter').length).toBeGreaterThan(0)
  })

  it('added/removed 缺失时从 diff 正文统计，不显示 0/0', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'sessions.fileChanges')
    rpc.call.mockResolvedValue({
      ok: true,
      changes: [{ path: 'src/a.ts', kind: 'modify', diff: DIFF }],
    })
    const host = await mountViewer({ sessionKey: 's1', initialOpenPath: 'src/a.ts' })
    expect(host.querySelector('.fcv__added')?.textContent).toBe('+2')
    expect(host.querySelector('.fcv__removed')?.textContent).toBe('-1')
  })

  it('「在编辑器打开」派发工作台事件（scopeId = 会话，path = 文件）', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'sessions.fileChanges')
    rpc.call.mockResolvedValue({
      ok: true,
      changes: [{ path: 'src/a.ts', kind: 'modify', diff: DIFF }],
    })
    const host = await mountViewer({ sessionKey: 's1' })

    const listener = vi.fn()
    window.addEventListener('marguerite:open-editor', listener)
    host.querySelector<HTMLButtonElement>('[data-testid="file-change-open"]')?.click()
    window.removeEventListener('marguerite:open-editor', listener)

    expect(listener).toHaveBeenCalledTimes(1)
    const detail = (listener.mock.calls[0][0] as CustomEvent).detail
    expect(detail).toMatchObject({ scopeId: 's1', path: 'src/a.ts' })
  })

  it('truncated 时给出截断说明', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'sessions.fileChanges')
    rpc.call.mockResolvedValue({
      ok: true,
      changes: [{ path: 'src/a.ts', kind: 'modify', diff: DIFF, truncated: true }],
    })
    const host = await mountViewer({ sessionKey: 's1', initialOpenPath: 'src/a.ts' })
    const note = host.querySelector('[data-testid="file-change-truncated"]')
    expect(note).toBeTruthy()
    expect(note?.textContent).toContain(String(DIFF.length))
  })

  it('后端无 sessions.fileChanges 时降级到 artifacts.list：列文件但不显示 diff 正文', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'artifacts.list')
    rpc.call.mockResolvedValue({
      artifacts: [{ name: 'report.md', path: 'out/report.md', kind: 'created' }],
    })
    const host = await mountViewer({ sessionKey: 's1' })

    expect(host.querySelector('[data-testid="file-change-degraded"]')).toBeTruthy()
    const rows = host.querySelectorAll('[data-testid="file-change-row"]')
    expect(rows).toHaveLength(1)
    expect(rows[0].textContent).toContain('out/report.md')
    expect(host.querySelector('[data-testid="file-change-toggle"]')).toBeNull()
    expect(host.querySelector('[data-testid="file-change-diff"]')).toBeNull()
  })

  it('空改动用预置空态键；调用失败用预置失败态键', async () => {
    rpc.supportsMethod.mockImplementation((method: string) => method === 'sessions.fileChanges')
    rpc.call.mockResolvedValueOnce({ ok: true, changes: [] })
    const emptyHost = await mountViewer({ sessionKey: 's1' })
    expect(emptyHost.querySelector('[data-testid="file-change-empty"]')).toBeTruthy()

    rpc.call.mockRejectedValueOnce(new Error('boom'))
    const failedHost = await mountViewer({ sessionKey: 's2' })
    expect(failedHost.querySelector('[data-testid="file-change-failed"]')).toBeTruthy()
  })
})
