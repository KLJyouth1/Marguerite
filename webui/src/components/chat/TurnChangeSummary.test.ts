// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, ref, type App } from 'vue'
import i18n from '@/i18n'
import { useToasts } from '@/composables/useToasts'
import TurnChangeSummary from './TurnChangeSummary.vue'

const rpc = vi.hoisted(() => ({
  isConnected: true,
  supportsMethod: vi.fn(),
  call: vi.fn(),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => rpc,
}))

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
  useToasts().toasts.value = []
})

beforeEach(() => {
  rpc.supportsMethod.mockReset()
  rpc.call.mockReset()
  rpc.supportsMethod.mockImplementation((method: string) =>
    method === 'sessions.fileChanges' || method === 'workspace.file_changes.list')
})

async function settle() {
  await nextTick()
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

async function mountSummary(
  since: string | number | null = 1000,
  listeners: { onView?: () => void; onReverted?: (count: number) => void } = {},
) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(TurnChangeSummary, { sessionKey: 's1', since, ...listeners }),
  })
  app.use(i18n)
  app.mount(host)
  apps.push(app)
  await settle()
  return host
}

function buttonByText(host: ParentNode, text: string): HTMLButtonElement | null {
  return [...host.querySelectorAll<HTMLButtonElement>('button')]
    .find(button => button.textContent?.trim() === text) ?? null
}

function turnChanges(host: ParentNode): HTMLElement | null {
  return host.querySelector<HTMLElement>('[data-testid="turn-changes"]')
}

function serveChanges(rows: Array<Record<string, unknown>>) {
  rpc.call.mockImplementation(async (method: string) => {
    if (method === 'sessions.fileChanges') return { ok: true, changes: rows }
    if (method === 'workspace.file_changes.list') return { items: [] }
    return { ok: true, reverted: true }
  })
}

describe('TurnChangeSummary', () => {
  it('渲染本回合的文件数与增删行数，展开后逐文件列出', async () => {
    serveChanges([
      { path: 'src/a.ts', kind: 'modify', tool: 'edit_file', added: 2, removed: 1, ts: 1001 },
    ])
    const host = await mountSummary()

    const bar = turnChanges(host)
    expect(bar).toBeTruthy()
    expect(bar?.textContent).toContain(i18n.global.t('chat.turnChanges.titleOne'))
    expect(bar?.textContent).toContain('+2')
    expect(bar?.textContent).toContain('−1')

    const toggle = bar?.querySelector<HTMLButtonElement>('.turn-changes__toggle')
    expect(toggle?.getAttribute('aria-expanded')).toBe('false')
    toggle?.click()
    await settle()

    expect(toggle?.getAttribute('aria-expanded')).toBe('true')
    const rows = host.querySelectorAll('.turn-changes__row')
    expect(rows).toHaveLength(1)
    expect(rows[0].textContent).toContain('src/a.ts')
    expect(rows[0].textContent).toContain(i18n.global.t('chat.fileChange.modify'))
  })

  it('「查看差异」只发事件，不自行打开面板', async () => {
    serveChanges([{ path: 'src/a.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 }])
    const onView = vi.fn()
    const host = await mountSummary(1000, { onView })

    buttonByText(host, i18n.global.t('chat.turnChanges.view'))?.click()
    await settle()

    expect(onView).toHaveBeenCalledTimes(1)
    expect(turnChanges(host)).toBeTruthy()
  })

  it('「全部保留」关闭汇总', async () => {
    serveChanges([{ path: 'src/a.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 }])
    const host = await mountSummary()

    buttonByText(host, i18n.global.t('chat.turnChanges.keep'))?.click()
    await settle()

    expect(turnChanges(host)).toBeNull()
  })

  it('上报本回合统计，并允许外部直接打开两段式撤销确认', async () => {
    serveChanges([{ path: 'src/a.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 }])
    const onStats = vi.fn()
    const summaryRef = ref<{ requestRevert: () => void } | null>(null)
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp({
      render: () => h(TurnChangeSummary, {
        ref: summaryRef,
        sessionKey: 's1',
        since: 1000,
        onStats,
      }),
    })
    app.use(i18n)
    app.mount(host)
    apps.push(app)
    await settle()

    expect(onStats).toHaveBeenLastCalledWith({ files: 1, canRevert: true })

    summaryRef.value?.requestRevert()
    await settle()

    expect(host.textContent).toContain(i18n.global.t('chat.turnChanges.confirmHint'))
    expect(rpc.call).not.toHaveBeenCalledWith('workspace.file_changes.revert', expect.anything())
  })

  it('撤销需要两步确认，成功后移除汇总并提示', async () => {
    serveChanges([{ path: 'made.ts', kind: 'create', added: 3, removed: 0, ts: 1001 }])
    const onReverted = vi.fn()
    const host = await mountSummary(1000, { onReverted })

    buttonByText(host, i18n.global.t('chat.turnChanges.revert'))?.click()
    await settle()
    // 第二步确认出现前不得调用回滚
    expect(rpc.call).not.toHaveBeenCalledWith('workspace.file_changes.revert', expect.anything())
    expect(host.textContent).toContain(i18n.global.t('chat.turnChanges.confirmHint'))

    host.querySelector<HTMLButtonElement>('[data-testid="turn-changes-confirm"]')?.click()
    await settle()

    expect(rpc.call).toHaveBeenCalledWith('workspace.file_changes.revert', {
      path: 'made.ts',
      kind: 'create',
      before: '',
    })
    expect(turnChanges(host)).toBeNull()
    expect(onReverted).toHaveBeenCalledWith(1)
    expect(useToasts().toasts.value.some(toast =>
      toast.message === i18n.global.t('chat.turnChanges.reverted', { n: 1 }))).toBe(true)
  })

  it('撤销部分失败时保留汇总，并如实说明原因', async () => {
    rpc.call.mockImplementation(async (method: string) => {
      if (method === 'sessions.fileChanges') {
        return {
          ok: true,
          changes: [
            { path: 'made.ts', kind: 'create', added: 1, removed: 0, ts: 1002 },
            { path: 'edited.ts', kind: 'modify', added: 1, removed: 1, ts: 1001 },
          ],
        }
      }
      if (method === 'workspace.file_changes.list') return { items: [] }
      return { ok: true, reverted: true }
    })
    const host = await mountSummary()

    // 展开后再撤销，才能确认「失败行保留、成功行移除」
    host.querySelector<HTMLButtonElement>('.turn-changes__toggle')?.click()
    await settle()

    buttonByText(host, i18n.global.t('chat.turnChanges.revert'))?.click()
    await settle()
    host.querySelector<HTMLButtonElement>('[data-testid="turn-changes-confirm"]')?.click()
    await settle()

    expect(turnChanges(host)).toBeTruthy()
    const alert = host.querySelector('[role="alert"]')
    expect(alert?.textContent).toContain(i18n.global.t('chat.turnChanges.noSnapshot', { path: 'edited.ts' }))
    // 失败行保留、成功行移除
    expect(host.querySelectorAll('.turn-changes__row')).toHaveLength(1)
    expect(host.textContent).toContain('edited.ts')
    expect(host.textContent).not.toContain('made.ts')
    expect(useToasts().toasts.value.some(toast =>
      toast.tone === 'warn')).toBe(true)
  })

  it('后端没有 sessions.fileChanges 时整条汇总不渲染', async () => {
    rpc.supportsMethod.mockReturnValue(false)
    const host = await mountSummary()

    expect(turnChanges(host)).toBeNull()
    expect(rpc.call).not.toHaveBeenCalled()
  })

  it('since 为空时不发起调用', async () => {
    const host = await mountSummary(null)

    expect(turnChanges(host)).toBeNull()
    expect(rpc.call).not.toHaveBeenCalled()
  })
})
