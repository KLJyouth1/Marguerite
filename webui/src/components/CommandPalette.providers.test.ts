// @vitest-environment happy-dom
import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

import i18n from '@/i18n'
import CommandPalette from './CommandPalette.vue'
import { useActiveModeStore } from '@/stores/activeMode'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'
import { loadStoredOrchestration } from '@/composables/chat/orchestration'

const mock = vi.hoisted(() => {
  const calls: Array<{ method: string; params?: unknown }> = []
  const responses: Record<string, unknown> = {}
  const push = vi.fn(async () => undefined)
  const rpc = {
    state: 'connected',
    supportsMethod: (_method: string) => true,
    canManageProjectWorkspaces: true,
    canChooseProject: true,
    isLocalOwner: true,
    call: async (method: string, params?: unknown) => {
      calls.push({ method, params })
      const entry = responses[method]
      return typeof entry === 'function'
        ? (entry as (p?: unknown) => unknown)(params)
        : entry
    },
  }
  return { calls, responses, push, rpc }
})

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: mock.push }),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => mock.rpc,
}))

vi.mock('@/composables/useBgm', () => ({
  useBgm: () => ({ enabled: { value: false }, setEnabled: () => {} }),
}))

const pinia = createPinia()
setActivePinia(pinia)

const WORKSPACE = {
  id: 'ws-1',
  name: 'Alpha Studio',
  path: '/work/alpha',
  taskCount: 2,
  pinned: false,
  available: true,
}

describe('CommandPalette live switch providers', () => {
  let app: App<Element> | null = null

  beforeEach(() => {
    mock.calls.length = 0
    for (const key of Object.keys(mock.responses)) delete mock.responses[key]
    mock.push.mockClear()
    try { localStorage.clear() } catch {}
    i18n.global.locale.value = 'en'

    mock.responses['workspaces.list'] = { workspaces: [WORKSPACE] }
    mock.responses['workspaces.open'] = { workspace: WORKSPACE }
    mock.responses['config.get'] = {
      llm: { provider: 'openai', model: 'gpt-x' },
      llm_profile_rows: [
        { id: 'p1', provider: 'openai', model: 'gpt-x', label: 'Fast Cloud', kind: 'cloud', active: true },
      ],
    }
    mock.responses['models.list'] = {
      current: { provider: 'openai', model: 'gpt-x' },
      models: [
        { id: 'p1', label: 'Fast Cloud', provider: 'openai', model: 'gpt-x', kind: 'cloud' },
        { id: 'p2', label: 'Local Gem', provider: 'ollama', model: 'gemma:7b', kind: 'local' },
      ],
    }
    mock.responses['team.compositions.list'] = {
      items: [{ id: 'deck_craft_custom', name: 'deck_craft_custom', label: 'Deck Crew', goal: 'Slides' }],
    }
    mock.responses['agents.list'] = {
      agents: [
        { id: 'main', name: 'Main Agent' },
        { id: 'researcher', name: 'Researcher', description: 'Evidence first' },
      ],
    }
  })

  afterEach(() => {
    app?.unmount()
    app = null
    document.body.innerHTML = ''
    useActiveModeStore().mode = null
  })

  async function settle() {
    for (let i = 0; i < 4; i++) {
      await nextTick()
      await new Promise(resolve => setTimeout(resolve, 0))
    }
  }

  async function mountPalette() {
    const el = document.createElement('div')
    document.body.appendChild(el)
    app = createApp(CommandPalette, { open: true, recents: [] })
    app.use(pinia)
    app.use(i18n)
    app.mount(el)
    await settle()
    return el
  }

  async function search(el: Element, value: string) {
    const input = el.querySelector<HTMLInputElement>('.cmdp-search__input')!
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await settle()
  }

  function labels(el: Element): string[] {
    return Array.from(el.querySelectorAll('.cmdp-option__label')).map(node => node.textContent || '')
  }

  /** Option labels inside one rendered group (labels are followed by options). */
  function groupLabels(el: Element, group: string): string[] {
    const headings = Array.from(el.querySelectorAll('.cmdp-group-label'))
    const heading = headings.find(node => node.textContent === group)
    if (!heading) return []
    const out: string[] = []
    let node = heading.nextElementSibling
    while (node && !node.classList.contains('cmdp-group-label')) {
      if (node.classList.contains('cmdp-option')) {
        out.push(node.querySelector('.cmdp-option__label')?.textContent || '')
      }
      node = node.nextElementSibling
    }
    return out
  }

  function clickLabel(el: Element, label: string) {
    const option = Array.from(el.querySelectorAll<HTMLButtonElement>('.cmdp-option')).find(node =>
      node.querySelector('.cmdp-option__label')?.textContent === label,
    )
    expect(option, `option "${label}" should be listed`).toBeTruthy()
    option!.click()
  }

  it('lists workspaces and switches to one via workspaces.open', async () => {
    const el = await mountPalette()

    await search(el, 'ws')
    expect(labels(el)).toContain('Alpha Studio')
    expect(el.querySelector('.cmdp-group-label')?.textContent).toBe('Workspaces')

    clickLabel(el, 'Alpha Studio')
    await settle()

    expect(mock.calls.some(call => call.method === 'workspaces.open')).toBe(true)
    expect(mock.push).toHaveBeenCalledWith({
      path: '/chat/new',
      query: { agent: 'main', project: 'ws-1' },
    })
    expect(useFreshTaskDraft().request.value).toMatchObject({ agentId: 'main', workspaceId: 'ws-1' })
  })

  it('accepts the 工作区 alias for workspaces', async () => {
    const el = await mountPalette()
    await search(el, '工作区')
    expect(labels(el)).toContain('Alpha Studio')
  })

  it('shows only the five most recent workspaces, pinned first', async () => {
    const rows = [
      { id: 'w-pinned', name: 'Pinned Alpha', path: '/work/pinned', pinned: true, lastOpenedAt: '' },
      { id: 'w-bravo', name: 'Bravo', path: '/work/bravo', pinned: false, lastOpenedAt: '2026-10-26T10:00:00Z' },
      { id: 'w-charlie', name: 'Charlie', path: '/work/charlie', pinned: false, lastOpenedAt: '2026-10-25T10:00:00Z' },
      { id: 'w-delta', name: 'Delta', path: '/work/delta', pinned: false, lastOpenedAt: '2026-10-24T10:00:00Z' },
      { id: 'w-echo', name: 'Echo', path: '/work/echo', pinned: false, lastOpenedAt: '2026-10-23T10:00:00Z' },
      { id: 'w-foxtrot', name: 'Foxtrot', path: '/work/foxtrot', pinned: false, lastOpenedAt: '2026-10-22T10:00:00Z' },
    ]
    mock.responses['workspaces.list'] = { workspaces: rows }
    const el = await mountPalette()

    await search(el, 'ws')

    // W8.4: the palette is the "switch to a recent project" shortcut, not the
    // full management list (that is ProjectWorkspacePickerDialog). Pinned rows
    // sort first, then `lastOpenedAt` desc; the sixth-oldest falls off.
    expect(groupLabels(el, 'Workspaces')).toEqual(['Pinned Alpha', 'Bravo', 'Charlie', 'Delta', 'Echo'])
  })

  it('does not list workspaces when the gateway cannot open them', async () => {
    const original = mock.rpc.canChooseProject
    mock.rpc.canChooseProject = false
    try {
      const el = await mountPalette()
      await search(el, 'ws')
      expect(labels(el)).not.toContain('Alpha Studio')
      // Listing without `workspaces.open` would render a dead switch action, so
      // the provider must not even ask for the list.
      expect(mock.calls.some(call => call.method === 'workspaces.list')).toBe(false)
    } finally {
      mock.rpc.canChooseProject = original
    }
  })

  it('lists models from config.get/models.list and activates one', async () => {
    const el = await mountPalette()

    await search(el, 'model')
    expect(labels(el)).toContain('Local Gem')

    await search(el, '模型')
    expect(labels(el)).toEqual(expect.arrayContaining(['Fast Cloud', 'Local Gem']))
    // `config.get` and `models.list` both describe the same openai profile; the
    // provider must de-duplicate on `provider:model`, not render it twice.
    expect(labels(el).filter(label => label === 'Fast Cloud')).toHaveLength(1)

    clickLabel(el, 'Local Gem')
    await settle()

    expect(mock.calls.some(call =>
      call.method === 'onboarding.llmProfile.activate'
      // `models.list` rows carry `kind` → they are Rust profile rows, so the
      // unambiguous `profileId` is sent alongside provider/model.
      && JSON.stringify(call.params) === JSON.stringify({
        profileId: 'p2',
        providerId: 'ollama',
        model: 'gemma:7b',
      }),
    )).toBe(true)
  })

  it('lists teams (compositions + expert registry) and selects one for the next task', async () => {
    const el = await mountPalette()

    await search(el, 'team')
    expect(labels(el)).toContain('Deck Crew')
    expect(labels(el)).toContain('PPT 智造专家团')

    clickLabel(el, 'Deck Crew')
    await settle()

    expect(loadStoredOrchestration()).toMatchObject({
      form: 'team',
      teams: ['deck_craft_custom'],
      confirmed: false,
    })
    expect(mock.push).toHaveBeenCalledWith({ path: '/chat/new', query: { agent: 'main' } })
  })

  it('accepts the 团队 alias for teams', async () => {
    const el = await mountPalette()
    await search(el, '团队')
    expect(labels(el)).toContain('PPT 智造专家团')
  })

  it('lists agents and opens a new chat with that agent', async () => {
    const el = await mountPalette()

    await search(el, 'agent')
    expect(labels(el)).toContain('Researcher')

    clickLabel(el, 'Researcher')
    await settle()

    expect(useFreshTaskDraft().request.value).toMatchObject({ agentId: 'researcher' })
    expect(mock.push).toHaveBeenCalledWith({ path: '/chat/new', query: { agent: 'researcher' } })
  })

  it('degrades model rows to none when neither config.get nor models.list is supported', async () => {
    const original = mock.rpc.supportsMethod
    mock.rpc.supportsMethod = (method: string) => method !== 'config.get' && method !== 'models.list'
    try {
      const el = await mountPalette()
      await search(el, 'model')
      expect(labels(el)).not.toContain('Fast Cloud')
      expect(labels(el)).not.toContain('Local Gem')
      expect(mock.calls.some(call =>
        call.method === 'config.get' || call.method === 'models.list',
      )).toBe(false)
    } finally {
      mock.rpc.supportsMethod = original
    }
  })

  it('keeps the static expert catalogue when live compositions are unavailable', async () => {
    const original = mock.rpc.supportsMethod
    mock.rpc.supportsMethod = (method: string) => method !== 'team.compositions.list'
    try {
      const el = await mountPalette()
      await search(el, 'team')
      // The gateway cannot list custom compositions, so no live row may appear…
      expect(labels(el)).not.toContain('Deck Crew')
      // …but the built-in expert catalogue needs no RPC and must stay usable.
      expect(labels(el)).toContain('PPT 智造专家团')
      expect(mock.calls.some(call => call.method === 'team.compositions.list')).toBe(false)

      clickLabel(el, 'PPT 智造专家团')
      await settle()
      expect(loadStoredOrchestration()).toMatchObject({ form: 'expert', teams: ['deck_craft'], confirmed: false })
    } finally {
      mock.rpc.supportsMethod = original
    }
  })

  it('degrades agent rows to none when agents.list is unsupported', async () => {
    const original = mock.rpc.supportsMethod
    mock.rpc.supportsMethod = (method: string) => method !== 'agents.list'
    try {
      const el = await mountPalette()
      await search(el, 'agent')
      expect(labels(el)).not.toContain('Researcher')
      expect(mock.calls.some(call => call.method === 'agents.list')).toBe(false)
    } finally {
      mock.rpc.supportsMethod = original
    }
  })

  it('keeps keyboard navigation over live results (ArrowDown + Enter)', async () => {
    mock.responses['workspaces.list'] = {
      workspaces: [
        WORKSPACE,
        { id: 'ws-2', name: 'Beta Lab', path: '/work/beta', pinned: false, available: true },
      ],
    }
    const el = await mountPalette()
    await search(el, 'ws')

    const input = el.querySelector<HTMLInputElement>('.cmdp-search__input')!
    expect(el.querySelector('.cmdp-option.is-active .cmdp-option__label')?.textContent)
      .toBe('Alpha Studio')

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    await nextTick()
    expect(el.querySelector('.cmdp-option.is-active .cmdp-option__label')?.textContent)
      .toBe('Beta Lab')

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()

    expect(mock.calls.some(call =>
      call.method === 'workspaces.open'
      && JSON.stringify(call.params) === JSON.stringify({ path: '/work/beta', trusted: true }),
    )).toBe(true)
    expect(useFreshTaskDraft().request.value).toMatchObject({ agentId: 'main', workspaceId: 'ws-2' })
  })

  it('degrades to no live results when the gateway declares no switch methods', async () => {
    const original = mock.rpc.supportsMethod
    mock.rpc.supportsMethod = () => false
    try {
      const el = await mountPalette()
      await search(el, 'ws')
      expect(labels(el)).not.toContain('Alpha Studio')
      // …and the Workspaces group has no reason to render at all.
      expect(Array.from(el.querySelectorAll('.cmdp-group-label')).map(node => node.textContent))
        .not.toContain('Workspaces')
    } finally {
      mock.rpc.supportsMethod = original
    }
  })
})
