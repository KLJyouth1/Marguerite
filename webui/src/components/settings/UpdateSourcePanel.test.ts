// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { App } from 'vue'

const mounted: App[] = []

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
  await new Promise(resolve => setTimeout(resolve, 10))
}

const CONFIG_VIEW = {
  ok: true,
  configured: true,
  repo: 'KLJyouth1/Marguerite',
  branch: 'main',
  currentVersion: '0.2.16',
  checkedAt: null as string | null,
  syncedAt: null as string | null,
  content: { skills: 3, plugins: 1 },
}

async function mountPanel(
  options: {
    methods?: string[]
    call?: (method: string, params?: Record<string, unknown>) => unknown
    confirm?: () => Promise<boolean>
  } = {},
) {
  vi.resetModules()
  document.body.innerHTML = ''
  const methods =
    options.methods ?? ['update.config.get', 'update.config.set', 'update.check', 'update.syncContent']
  const call = vi.fn(
    options.call ??
      (async (method: string) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.config.set') return { ...CONFIG_VIEW }
        return { ok: false, code: 'UNEXPECTED', message: method }
      }),
  )
  const rpc = {
    waitForConnection: vi.fn(async () => {}),
    supportsMethod: vi.fn((method: string) => methods.includes(method)),
    markMethodUnavailable: vi.fn(),
    call,
  }
  vi.doMock('@/stores/rpc', () => ({ useRpcStore: () => rpc }))
  const confirm = vi.fn(options.confirm ?? (async () => true))
  vi.doMock('@/composables/useConfirm', () => ({
    useConfirm: () => ({ confirm, confirmState: { value: null }, resolveConfirm: vi.fn() }),
  }))

  const { createApp, nextTick } = await import('vue')
  const i18n = (await import('@/i18n')).default
  i18n.global.locale.value = 'en'
  const Component = (await import('./UpdateSourcePanel.vue')).default
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Component)
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  await settle()
  await nextTick()
  return { el, call, confirm }
}

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
})

describe('UpdateSourcePanel', () => {
  it('shows the saved source plus live version and local content counts', async () => {
    const { el } = await mountPanel()
    const text = el.textContent ?? ''
    expect(text).toContain('0.2.16')
    expect(text).toContain('KLJyouth1/Marguerite')
    expect(text).toContain('main')
    expect(text).toContain('3')
    expect(text).toContain('1')
    expect(el.querySelector<HTMLInputElement>('[data-testid="update-source-repo"]')!.value).toBe(
      'KLJyouth1/Marguerite',
    )
  })

  it('rejects an invalid repository locally without calling the gateway', async () => {
    const { el, call } = await mountPanel()
    const repo = el.querySelector<HTMLInputElement>('[data-testid="update-source-repo"]')!
    repo.value = 'not-a-repo'
    repo.dispatchEvent(new Event('input'))
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-save"]')!.click()
    await settle()
    expect(el.textContent).toContain('must look like owner/name')
    expect(call).not.toHaveBeenCalledWith('update.config.set', expect.anything(), expect.anything())
  })

  it('saves a valid repository through update.config.set', async () => {
    const { el, call } = await mountPanel({
      call: async (method: string, params?: Record<string, unknown>) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.config.set') {
          return { ...CONFIG_VIEW, repo: params?.repo, branch: params?.branch }
        }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })
    const repo = el.querySelector<HTMLInputElement>('[data-testid="update-source-repo"]')!
    repo.value = 'owner/name'
    repo.dispatchEvent(new Event('input'))
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-save"]')!.click()
    await settle()
    expect(call).toHaveBeenCalledWith(
      'update.config.set',
      { repo: 'owner/name', branch: 'main' },
      expect.objectContaining({ queue: false }),
    )
    expect(el.textContent).toContain('Source saved')
  })

  it('renders the honest reason when the repository has no release', async () => {
    const { el } = await mountPanel({
      call: async (method: string) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.check') return { ok: false, code: 'NO_RELEASES', message: 'no release' }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-check"]')!.click()
    await settle()
    const error = el.querySelector('[data-testid="update-source-check-error"]')
    expect(error?.textContent).toContain('no published release')
    expect(el.textContent).not.toContain('A newer release is available.')
  })

  it('surfaces network failures instead of a fake success', async () => {
    const { el } = await mountPanel({
      call: async (method: string) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.check') return { ok: false, code: 'NET_ERROR', message: 'boom' }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-check"]')!.click()
    await settle()
    expect(el.querySelector('[data-testid="update-source-check-error"]')?.textContent).toContain(
      'Network request failed: boom',
    )
  })

  it('dry-runs first, then applies only after an explicit confirmation', async () => {
    const calls: Array<{ method: string; params?: Record<string, unknown> }> = []
    const { el, confirm } = await mountPanel({
      call: async (method: string, params?: Record<string, unknown>) => {
        calls.push({ method, params })
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.syncContent' && params?.apply === true) {
          return {
            ok: true,
            applied: true,
            added: [{ kind: 'skill', name: 'b', path: 'skills/b/SKILL.md' }],
            changed: [],
            unchanged: [],
            total: 1,
          }
        }
        if (method === 'update.syncContent') {
          return {
            ok: true,
            applied: false,
            added: [{ kind: 'skill', name: 'b', path: 'skills/b/SKILL.md' }],
            changed: [{ kind: 'skill', name: 'a', path: 'skills/a/SKILL.md' }],
            unchanged: [{ kind: 'plugin', name: 'c', path: 'plugins/c/manifest.json' }],
            total: 3,
          }
        }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })

    el.querySelector<HTMLButtonElement>('[data-testid="update-source-sync"]')!.click()
    await settle()
    expect(calls.filter(c => c.method === 'update.syncContent')).toHaveLength(1)
    expect(calls.find(c => c.method === 'update.syncContent')?.params).toEqual({ apply: false })
    expect(el.textContent).toContain('3 items in manifest')
    expect(el.textContent).toContain('skill/a')

    el.querySelector<HTMLButtonElement>('[data-testid="update-source-apply"]')!.click()
    await settle()
    expect(confirm).toHaveBeenCalledTimes(1)
    const syncCalls = calls.filter(c => c.method === 'update.syncContent')
    expect(syncCalls).toHaveLength(2)
    expect(syncCalls[1].params).toEqual({ apply: true })
    expect(el.textContent).toContain('Content updated: 1 new, 0 changed.')
  })

  it('does not write anything when the confirmation is cancelled', async () => {
    const calls: Array<Record<string, unknown> | undefined> = []
    const { el } = await mountPanel({
      confirm: async () => false,
      call: async (method: string, params?: Record<string, unknown>) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.syncContent') {
          calls.push(params)
          return {
            ok: true,
            applied: false,
            added: [{ kind: 'skill', name: 'b', path: 'skills/b/SKILL.md' }],
            changed: [],
            unchanged: [],
            total: 1,
          }
        }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-sync"]')!.click()
    await settle()
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-apply"]')!.click()
    await settle()
    expect(calls).toEqual([{ apply: false }])
    expect(el.textContent).not.toContain('Content updated:')
  })

  it('reports the honest error when the manifest is missing', async () => {
    const { el } = await mountPanel({
      call: async (method: string) => {
        if (method === 'update.config.get') return { ...CONFIG_VIEW }
        if (method === 'update.syncContent') return { ok: false, code: 'MANIFEST_MISSING' }
        return { ok: false, code: 'UNEXPECTED' }
      },
    })
    el.querySelector<HTMLButtonElement>('[data-testid="update-source-sync"]')!.click()
    await settle()
    expect(el.querySelector('[data-testid="update-source-sync-error"]')?.textContent).toContain(
      'no update-manifest.json',
    )
  })

  it('stays disabled when the gateway does not expose the update methods', async () => {
    const { el, call } = await mountPanel({ methods: [] })
    expect(el.querySelector('[data-testid="update-source-unsupported"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="update-source-check"]')).toBeNull()
    expect(call).not.toHaveBeenCalled()
  })
})
