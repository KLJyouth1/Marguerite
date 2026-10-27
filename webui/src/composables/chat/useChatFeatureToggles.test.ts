import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useChatFeatureToggles } from './useChatFeatureToggles'
import source from './useChatFeatureToggles.ts?raw'
import i18n from '@/i18n'
import en from '@/locales/en.json'
import { useToasts } from '@/composables/useToasts'
import type { RpcCallOptions } from '@/lib/rpc'
import type { ModelRoutingMode } from '@/types/modelRouting'

type RpcResult = Record<string, unknown> | Error | Promise<unknown>

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

function createHarness(options: {
  configGetResults?: RpcResult[]
  routingGetResults?: RpcResult[]
  patchResults?: RpcResult[]
  readCallOptions?: RpcCallOptions
} = {}) {
  const configGetResults = [...(options.configGetResults ?? [{}])]
  const routingGetResults = [...(options.routingGetResults ?? [])]
  const patchResults = [...(options.patchResults ?? [])]
  const eventHandlers = new Map<string, (payload: unknown) => void>()
  const waitForConnection = vi.fn(async () => {})
  const setGlobalElevatedMode = vi.fn()
  const loadCurrentSessionUsage = vi.fn()
  const call = vi.fn(async (method: string, _params?: Record<string, unknown>): Promise<unknown> => {
    if (method === 'config.get') {
      const result = configGetResults.shift() ?? {}
      if (result instanceof Error) throw result
      return await Promise.resolve(result)
    }
    if (method === 'models.routing.get') {
      const result = routingGetResults.shift()
      if (result === undefined) throw new Error('canonical routing unavailable')
      if (result instanceof Error) throw result
      return await Promise.resolve(result)
    }
    if (method === 'config.patch.safe' || method === 'models.routing.set') {
      const result = patchResults.shift()
      if (result instanceof Error) throw result
      await Promise.resolve(result)
      return { ok: true }
    }
    throw new Error(`Unexpected RPC method: ${method}`)
  })
  const rpc = {
    waitForConnection,
    call: call as <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>,
    on: vi.fn((event: string, handler: (payload: unknown) => void) => {
      eventHandlers.set(event, handler)
      return () => eventHandlers.delete(event)
    }),
  }
  const api = useChatFeatureToggles({
    rpc,
    readCallOptions: options.readCallOptions,
    setGlobalElevatedMode,
    loadCurrentSessionUsage,
  })
  return {
    api,
    rpc: { waitForConnection, call, on: rpc.on },
    emit: (event: string, payload: unknown) => eventHandlers.get(event)?.(payload),
    setGlobalElevatedMode,
    loadCurrentSessionUsage,
  }
}

function patchCalls(rpc: ReturnType<typeof createHarness>['rpc']) {
  return rpc.call.mock.calls.filter(([method]) => method === 'config.patch.safe')
}

function routingCalls(rpc: ReturnType<typeof createHarness>['rpc']) {
  return rpc.call.mock.calls.filter(([method]) => method === 'models.routing.set')
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('useChatFeatureToggles coding mode', () => {
  it('reads enabled coding mode from backend config', async () => {
    const readCallOptions: RpcCallOptions = {
      timeoutMs: 2_000,
      timeoutAction: 'reconnect',
      abortAction: 'reconnect',
    }
    const { api, rpc } = createHarness({
      configGetResults: [{ skills: { coding_mode: true } }],
      readCallOptions,
    })

    await api.loadFeatureToggles()

    expect(api.codingModeEnabled.value).toBe(true)
    expect(rpc.waitForConnection).toHaveBeenCalledWith(
      2_000,
      undefined,
      {
        timeoutAction: 'reconnect',
        abortAction: 'reconnect',
      },
    )
    expect(rpc.call).toHaveBeenCalledWith(
      'config.get',
      undefined,
      readCallOptions,
    )
  })

  it.each([
    {},
    { skills: {} },
  ])('defaults missing coding mode to off for %j', async (config) => {
    const { api } = createHarness({
      configGetResults: [config],
    })

    await api.loadFeatureToggles()

    expect(api.codingModeEnabled.value).toBe(false)
  })

  it('writes coding mode on with the safe backend patch path', async () => {
    const { api, rpc } = createHarness({
      configGetResults: [{ skills: { coding_mode: true } }],
    })

    await api.setCodingModeEnabled(true)

    expect(rpc.call).toHaveBeenCalledWith('config.patch.safe', {
      patches: { 'skills.coding_mode': true },
    })
  })

  it('writes coding mode off with the safe backend patch path', async () => {
    const { api, rpc } = createHarness({
      configGetResults: [{ skills: { coding_mode: false } }],
    })

    await api.setCodingModeEnabled(false)

    expect(rpc.call).toHaveBeenCalledWith('config.patch.safe', {
      patches: { 'skills.coding_mode': false },
    })
  })

  it('strictly reloads backend config after a successful write', async () => {
    const { api, rpc } = createHarness({
      configGetResults: [{ skills: { coding_mode: true } }],
    })

    await api.setCodingModeEnabled(true)

    const calls = rpc.call.mock.calls
    const patchIndex = calls.findIndex(([method]) => method === 'config.patch.safe')
    const getIndex = calls.findIndex(([method], index) => index > patchIndex && method === 'config.get')
    expect(patchIndex).toBeGreaterThanOrEqual(0)
    expect(getIndex).toBeGreaterThan(patchIndex)
    expect(api.codingModeEnabled.value).toBe(true)
  })

  it('applies the strict post-patch config through the shared feature mapping', async () => {
    const { api, setGlobalElevatedMode } = createHarness({
      configGetResults: [{
        skills: { coding_mode: true },
        squilla_router: { enabled: true, rollout_phase: 'full' },
        permissions: { default_mode: 'bypass' },
      }],
    })

    await api.setCodingModeEnabled(true)

    expect(api.codingModeEnabled.value).toBe(true)
    expect(api.routerEnabled.value).toBe(true)
    expect(setGlobalElevatedMode).toHaveBeenCalledWith('bypass')
  })

  it('keeps coding mode backend-confirmed while a write is pending', async () => {
    const pendingPatch = deferred<void>()
    const { api } = createHarness({
      patchResults: [pendingPatch.promise],
      configGetResults: [{ skills: { coding_mode: true } }],
    })

    const write = api.setCodingModeEnabled(true)
    await Promise.resolve()

    expect(api.codingModeSettingsBusy.value).toBe(true)
    expect(api.codingModeEnabled.value).toBe(false)

    pendingPatch.resolve(undefined)
    await write
    expect(api.codingModeEnabled.value).toBe(true)
  })

  it('rolls back when the backend patch fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { api } = createHarness({
      patchResults: [new Error('patch failed')],
    })

    await api.setCodingModeEnabled(true)

    expect(api.codingModeEnabled.value).toBe(false)
    expect(warn).toHaveBeenCalledWith('Failed to update Coding mode:', 'patch failed')
  })

  it('rolls back when post-patch config reload fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { api } = createHarness({
      configGetResults: [new Error('reload failed')],
    })

    await api.setCodingModeEnabled(true)

    expect(api.codingModeEnabled.value).toBe(false)
    expect(warn).toHaveBeenCalledWith('Failed to update Coding mode:', 'reload failed')
  })

  it('uses the post-patch backend value as authoritative', async () => {
    const { api } = createHarness({
      configGetResults: [{ skills: { coding_mode: false } }],
    })

    await api.setCodingModeEnabled(true)

    expect(api.codingModeEnabled.value).toBe(false)
  })

  it('prevents overlapping coding mode writes while busy', async () => {
    const pendingPatch = deferred<void>()
    const { api, rpc } = createHarness({
      patchResults: [pendingPatch.promise],
      configGetResults: [{ skills: { coding_mode: true } }],
    })

    const firstWrite = api.setCodingModeEnabled(true)
    await api.setCodingModeEnabled(false)
    await Promise.resolve()

    expect(patchCalls(rpc)).toHaveLength(1)
    expect(rpc.call).toHaveBeenCalledWith('config.patch.safe', {
      patches: { 'skills.coding_mode': true },
    })

    pendingPatch.resolve(undefined)
    await firstWrite
  })

  it('does not persist coding mode through browser storage APIs', () => {
    const setterStart = source.indexOf('async function setCodingModeEnabled')
    const setterEnd = source.indexOf('function bindFeatureRefresh', setterStart)
    const setterSource = source.slice(setterStart, setterEnd)

    expect(setterSource).toContain('skills.coding_mode')
    expect(setterSource).not.toMatch(/localStorage|sessionStorage/)
  })
})

describe('useChatFeatureToggles router visual effects', () => {
  it('persists router visual effects without a legacy browser global', () => {
    const setItem = vi.fn()
    vi.stubGlobal('localStorage', {
      getItem: vi.fn(() => null),
      setItem,
    })
    const { api } = createHarness()

    api.setRouterVisualEffectsEnabled(false)

    expect(api.routerVisualEffectsEnabled.value).toBe(false)
    expect(setItem).toHaveBeenCalledWith('marguerite.routerFx', JSON.stringify({
      enabled: false,
      variant: 'default',
    }))
    expect(source).not.toContain('SavingsFX')
  })
})

describe('useChatFeatureToggles model routing mode', () => {
  it('uses the canonical routing snapshot over legacy config inference', async () => {
    const { api } = createHarness({
      configGetResults: [{ squilla_router: { enabled: true, rollout_phase: 'full' } }],
      routingGetResults: [{ mode: 'direct' }],
    })

    await api.loadFeatureToggles()

    expect(api.modelRoutingMode.value).toBe('off')
  })

  it('applies Gateway routing changes live and unsubscribes on cleanup', async () => {
    vi.stubGlobal('document', {
      visibilityState: 'visible',
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    vi.stubGlobal('window', {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    const { api, rpc, emit } = createHarness()
    const cleanup = api.bindFeatureRefresh()

    // P3 打通：后端统一发 `models.changed`（原 models.routing.changed 后端从未发布）。
    emit('models.changed', { mode: 'ensemble', selection_mode: 'router_dynamic' })
    expect(api.modelRoutingMode.value).toBe('llm_ensemble')
    expect(api.llmEnsembleSelectionMode.value).toBe('router_dynamic')

    cleanup()
    expect(rpc.on).toHaveBeenCalledWith('models.changed', expect.any(Function))
    emit('models.changed', { mode: 'direct' })
    expect(api.modelRoutingMode.value).toBe('llm_ensemble')
  })

  it('refreshes toggles when models.changed carries no routing snapshot', async () => {
    vi.stubGlobal('document', {
      visibilityState: 'visible',
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    vi.stubGlobal('window', {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    const { api, rpc, emit } = createHarness()
    const cleanup = api.bindFeatureRefresh()
    const before = rpc.call.mock.calls.length

    // 非路由变更（如 llm_profile_upsert）：只重读 toggles，不应用路由快照。
    emit('models.changed', { reason: 'llm_profile_upsert', provider_id: 'p' })
    await Promise.resolve()

    expect(api.modelRoutingMode.value).toBe('off')
    expect(rpc.call.mock.calls.length).toBeGreaterThan(before)

    cleanup()
    vi.unstubAllGlobals()
  })

  it('derives bloom_ensemble from a gateway bloom snapshot', async () => {
    vi.stubGlobal('document', {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    vi.stubGlobal('window', {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
    const { api, emit } = createHarness()
    const cleanup = api.bindFeatureRefresh()

    emit('models.changed', { mode: 'bloom', selection_mode: 'router_dynamic' })
    expect(api.modelRoutingMode.value).toBe('bloom_ensemble')
    expect(api.llmEnsembleEnabled.value).toBe(true)
    expect(api.routerEnabled.value).toBe(true)

    cleanup()
    vi.unstubAllGlobals()
  })

  it.each([
    [{}, 'off', false, false],
    [{ squilla_router: { enabled: true, rollout_phase: 'observe' } }, 'off', false, false],
    [{ squilla_router: { enabled: true, rollout_phase: 'full' } }, 'squilla_router', true, false],
    [{ squilla_router: { enabled: false }, llm_ensemble: { enabled: true } }, 'llm_ensemble', true, true],
    [{ squilla_router: { enabled: true, rollout_phase: 'full' }, llm_ensemble: { enabled: true } }, 'llm_ensemble', true, true],
    [{ squilla_router: { enabled: true, rollout_phase: 'full' }, llm_ensemble: { enabled: true, orchestration_mode: 'auto' } }, 'bloom_ensemble', true, true],
  ])('maps backend config %j to mode %s', async (config, mode, routerActive, ensembleActive) => {
    const { api } = createHarness({
      configGetResults: [config],
    })

    await api.loadFeatureToggles()

    expect(api.modelRoutingMode.value).toBe(mode)
    expect(api.routerEnabled.value).toBe(routerActive)
    expect(api.llmEnsembleEnabled.value).toBe(ensembleActive)
  })

  it.each<[ModelRoutingMode, string]>([
    ['off', 'direct'],
    ['squilla_router', 'router'],
    ['llm_ensemble', 'ensemble'],
    ['bloom_ensemble', 'bloom'],
  ])('writes %s through the canonical Gateway state machine', async (mode, gatewayMode) => {
    const { api, rpc } = createHarness({
      configGetResults: [{}, {}],
    })

    await api.loadFeatureToggles()
    await api.setModelRoutingMode(mode)

    expect(rpc.call).toHaveBeenCalledWith('models.routing.set', { mode: gatewayMode })
  })

  it('optimistically reflects the selected routing mode while a write is pending', async () => {
    const pendingPatch = deferred<void>()
    const { api } = createHarness({
      patchResults: [pendingPatch.promise],
      configGetResults: [{ squilla_router: { enabled: true, rollout_phase: 'full' }, llm_ensemble: { enabled: true } }],
    })

    const write = api.setModelRoutingMode('llm_ensemble')
    await Promise.resolve()

    expect(api.modelRoutingSettingsBusy.value).toBe(true)
    expect(api.modelRoutingMode.value).toBe('llm_ensemble')
    expect(api.routerEnabled.value).toBe(true)
    expect(api.llmEnsembleEnabled.value).toBe(true)

    pendingPatch.resolve(undefined)
    await write
    expect(api.modelRoutingMode.value).toBe('llm_ensemble')
  })

  it('rolls back model routing when the backend patch fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { api } = createHarness({
      configGetResults: [{ squilla_router: { enabled: true, rollout_phase: 'full' } }],
      patchResults: [new Error('patch failed')],
    })

    await api.loadFeatureToggles()
    await api.setModelRoutingMode('off')

    expect(api.modelRoutingMode.value).toBe('squilla_router')
    expect(warn).toHaveBeenCalledWith('Failed to update model routing:', 'patch failed')
  })

  it('uses the post-patch backend value as authoritative', async () => {
    const { api } = createHarness({
      configGetResults: [{ squilla_router: { enabled: false }, llm_ensemble: { enabled: false } }],
    })

    await api.setModelRoutingMode('llm_ensemble')

    expect(api.modelRoutingMode.value).toBe('off')
  })

  it('prevents overlapping model-routing writes while busy', async () => {
    const pendingPatch = deferred<void>()
    const { api, rpc } = createHarness({
      patchResults: [pendingPatch.promise],
      configGetResults: [{ squilla_router: { enabled: true, rollout_phase: 'full' }, llm_ensemble: { enabled: true } }],
    })

    const firstWrite = api.setModelRoutingMode('llm_ensemble')
    await api.setModelRoutingMode('off')
    await Promise.resolve()

    expect(routingCalls(rpc)).toHaveLength(1)
    expect(rpc.call).toHaveBeenCalledWith('models.routing.set', { mode: 'ensemble' })

    pendingPatch.resolve(undefined)
    await firstWrite
  })

  it('does not persist model routing through browser storage APIs', () => {
    const setterStart = source.indexOf('async function setModelRoutingMode')
    const setterEnd = source.indexOf('function bindFeatureRefresh', setterStart)
    const setterSource = source.slice(setterStart, setterEnd)

    expect(setterSource).toContain("options.rpc.call('models.routing.set'")
    expect(setterSource).not.toContain("options.rpc.call('config.patch.safe'")
    expect(setterSource).not.toMatch(/localStorage|sessionStorage/)
  })
})

// The routing setter reports its outcome to the user through pushToast(): a
// resolved "effective" line when the Gateway landed on the requested mode, and a
// "mismatch" line naming requested vs actual when it did not. Both keys are
// reached through the object-qualified `i18n.global.t('chat.modelRouting.*')`
// form, which the source i18n guard only learned to scan recently — so pin the
// *rendered* text here, not merely the key's existence. A typo'd key must fail
// the suite instead of leaking a raw `chat.modelRouting.*` path into the toast.
const { toasts, dismissToast } = useToasts()

function resolveEn(dottedKey: string): string {
  let node: unknown = en
  for (const part of dottedKey.split('.')) {
    node = (node as Record<string, unknown>)[part]
  }
  return node as string
}

function renderEn(dottedKey: string, params: Record<string, string>): string {
  return Object.entries(params).reduce(
    (text, [name, value]) => text.replace(`{${name}}`, value),
    resolveEn(dottedKey),
  )
}

describe('useChatFeatureToggles model routing feedback toasts', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en'
    toasts.value = []
  })

  afterEach(() => {
    for (const toast of [...toasts.value]) dismissToast(toast.id)
  })

  it('renders the localized "effective" copy once the requested mode is live', async () => {
    const { api } = createHarness({
      configGetResults: [{}, { llm_ensemble: { enabled: true } }],
    })
    await api.loadFeatureToggles()
    toasts.value = []

    await api.setModelRoutingMode('llm_ensemble')

    const last = toasts.value[toasts.value.length - 1]
    const expected = renderEn('chat.modelRouting.effective', { mode: 'llm_ensemble' })
    expect(last?.message).toBe(expected)
    expect(last?.message).toContain('llm_ensemble')
    expect(last?.message).not.toContain('chat.modelRouting')
    expect(last?.tone).toBe('ok')
  })

  it('renders the localized "mismatch" copy with requested/actual when the mode did not switch', async () => {
    const { api } = createHarness({
      configGetResults: [
        { llm_ensemble: { enabled: true } },
        { squilla_router: { enabled: false }, llm_ensemble: { enabled: false } },
      ],
    })
    await api.loadFeatureToggles()
    toasts.value = []

    await api.setModelRoutingMode('llm_ensemble')

    const last = toasts.value[toasts.value.length - 1]
    const expected = renderEn('chat.modelRouting.mismatch', {
      requested: 'llm_ensemble',
      actual: 'off',
    })
    expect(last?.message).toBe(expected)
    expect(last?.message).toContain('llm_ensemble')
    expect(last?.message).toContain('off')
    expect(last?.message).not.toContain('chat.modelRouting')
    expect(last?.tone).toBe('danger')
  })
})
