// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import ProvenanceDrawer from './ProvenanceDrawer.vue'

const rpcMock = vi.hoisted(() => ({
  supported: true,
  call: null as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    supportsMethod: () => rpcMock.supported,
    call: (...args: unknown[]) =>
      (rpcMock.call as unknown as (...a: unknown[]) => unknown)(...args),
  }),
}))

const mounted: App[] = []

function stubMatchMedia(reduced: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: reduced && query.includes('prefers-reduced-motion'),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }))
}

function mountDrawer(): HTMLElement {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(ProvenanceDrawer)
  app.use(i18n)
  app.mount(el)
  mounted.push(app)
  return el
}

// Alt+P → open → RPC promise settles; one macrotask covers the whole chain.
async function flush(): Promise<void> {
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

async function pressAltP(): Promise<void> {
  window.dispatchEvent(
    new KeyboardEvent('keydown', { key: 'p', altKey: true, bubbles: true, cancelable: true }),
  )
  await flush()
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  stubMatchMedia(false)
  rpcMock.supported = true
  rpcMock.call = vi.fn()
  document.body.innerHTML = ''
})

afterEach(() => {
  while (mounted.length) mounted.pop()?.unmount()
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  i18n.global.locale.value = 'en'
})

describe('ProvenanceDrawer', () => {
  it('renders the honest projection empty state when the gateway omits projection', async () => {
    rpcMock.call.mockResolvedValue({
      scope: 'global',
      workspace: null,
      pinned: [],
      preferences: [],
      facts: [{ text: 'Build uses Vite', source: 'repo.docs', score: 0.64 }],
      open_loops: [],
      budget: { max_chars: 800, used: 512 },
      truncated: false,
    })

    const el = mountDrawer()
    await pressAltP()

    expect(el.querySelector('[data-testid="provenance-drawer"]')).not.toBeNull()
    const empty = el.querySelector('[data-testid="provenance-projection-empty"]')
    expect(empty?.textContent).toContain('projection details')
    // Missing projection must never masquerade as "nothing was dropped".
    expect(el.querySelector('[data-testid="provenance-nothing-dropped"]')).toBeNull()
  })

  it('renders each dropped key with its localized reason', async () => {
    rpcMock.call.mockResolvedValue({
      scope: 'project',
      workspace: '/repo/marguerite',
      pinned: [],
      preferences: [],
      facts: [],
      open_loops: [],
      projection: {
        policy: 'l1_unpredictable_only',
        droppedCount: 2,
        dropped: [
          { key: 'repo.layout', reason: 'workspace_derivable' },
          { key: 'tenant.owners', reason: 'low_score' },
        ],
      },
      budget: { max_chars: 800, used: 120 },
      truncated: false,
    })

    const el = mountDrawer()
    await pressAltP()

    const rows = el.querySelectorAll('[data-testid="provenance-dropped"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].textContent).toContain('repo.layout')
    expect(rows[0].textContent).toContain('Derivable from the workspace')
    expect(rows[1].textContent).toContain('tenant.owners')
    expect(rows[1].textContent).toContain('Low score')
    expect(el.querySelector('[data-testid="provenance-projection-empty"]')).toBeNull()
  })

  it('toggles with the global Alt+P shortcut', async () => {
    rpcMock.call.mockResolvedValue({ scope: 'global', workspace: 'w', facts: [] })

    const el = mountDrawer()
    await pressAltP()
    expect(el.querySelector('[data-testid="provenance-drawer"]')).not.toBeNull()

    await pressAltP()
    expect(el.querySelector('[data-testid="provenance-drawer"]')).toBeNull()
  })

  it('closes on Escape', async () => {
    rpcMock.call.mockResolvedValue({ scope: 'global', workspace: 'w', facts: [] })

    const el = mountDrawer()
    await pressAltP()
    expect(el.querySelector('[data-testid="provenance-drawer"]')).not.toBeNull()

    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
    )
    await nextTick()

    expect(el.querySelector('[data-testid="provenance-drawer"]')).toBeNull()
  })

  it('marks the drawer as reduced-motion when the system asks for it', async () => {
    stubMatchMedia(true)
    rpcMock.call.mockResolvedValue({ scope: 'global', workspace: 'w', facts: [] })

    const el = mountDrawer()
    await pressAltP()

    const drawer = el.querySelector('[data-testid="provenance-drawer"]')
    expect(drawer?.classList.contains('prov-drawer--reduced')).toBe(true)
  })

  it('offers "forget" only on facts the gateway marks deletable, then re-reads', async () => {
    rpcMock.call
      .mockResolvedValueOnce({
        scope: 'project',
        workspace: '/repo',
        facts: [
          {
            text: 'own fact',
            source: 'project:a#cross_project_facts.jsonl',
            score: 0.9,
            id: 'fact:7',
            deletable: true,
          },
          { text: 'external fact', source: 'project:a#project_memory.md', score: 0.4 },
        ],
        projection: { policy: 'l2_workspace_scoped', droppedCount: 0, dropped: [] },
      })
      .mockResolvedValueOnce({
        scope: 'project',
        workspace: '/repo',
        facts: [{ text: 'external fact', source: 'project:a#project_memory.md', score: 0.4 }],
        projection: { policy: 'l2_workspace_scoped', droppedCount: 0, dropped: [] },
      })
      .mockResolvedValueOnce({ success: true, deleted: true, id: 'fact:7' })

    const el = mountDrawer()
    await pressAltP()

    const buttons = el.querySelectorAll('[data-testid="provenance-fact-delete"]')
    expect(buttons).toHaveLength(1)

    ;(buttons[0] as HTMLButtonElement).click()
    await flush()

    // 第一次是删除，第二次是重取简报（顺序证明没有走本地数组打补丁）
    expect(rpcMock.call).toHaveBeenNthCalledWith(1, 'assistant.brief.get', {})
    expect(rpcMock.call).toHaveBeenNthCalledWith(2, 'assistant.facts.delete', { id: 'fact:7' })
    expect(rpcMock.call).toHaveBeenNthCalledWith(3, 'assistant.brief.get', {})
  })
})
