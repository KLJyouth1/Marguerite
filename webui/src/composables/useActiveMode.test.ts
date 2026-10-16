// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

// Controllable RPC store double: captures the `on` handler so tests can fire a
// `session.event.mode.changed` frame, and records `mode.switch` calls.
const modeChangedHandlers: Array<(frame: unknown) => void> = []
/** `_state` listeners, so a test can simulate the socket becoming connected. */
const stateHandlers: Array<(s: string) => void> = []
/** Flipped by the "waits for the transport" test. */
let rpcConnected = true
const rpcCall = vi.fn(async (_method: string, params?: unknown) => {
  if (_method === 'mode.get') return { kind: 'bloom', mode: 'bloom', label: 'Bloom' }
  if (_method === 'mode.switch') {
    const p = params as { kind?: string }
    return { kind: p?.kind, mode: p?.kind, label: p?.kind }
  }
  return null
})

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    // Hydration is gated on the transport (it must not fire before the socket
    // is connected), so the double has to report a connected socket.
    get isConnected() {
      return rpcConnected
    },
    call: (method: string, params?: unknown) => rpcCall(method, params),
    on: (event: string, handler: (arg: never) => void) => {
      if (event === 'session.event.mode.changed') {
        modeChangedHandlers.push(handler as (frame: unknown) => void)
      }
      if (event === '_state') {
        stateHandlers.push(handler as unknown as (s: string) => void)
      }
      return () => {
        const a = modeChangedHandlers.indexOf(handler as (frame: unknown) => void)
        if (a >= 0) modeChangedHandlers.splice(a, 1)
        const b = stateHandlers.indexOf(handler as unknown as (s: string) => void)
        if (b >= 0) stateHandlers.splice(b, 1)
      }
    },
  }),
}))

describe('useActiveMode (upgrade-masterplan §5)', () => {
  beforeEach(() => {
    // Isolation matters here for two reasons:
    //  1. the active-mode state lives in a Pinia store (cross-chunk sharing), so
    //     each test needs a fresh pinia;
    //  2. `initActiveMode()` is guarded by a module-level `initialised` flag AND
    //     kicks off a `mode.get` promise. Without resetModules the previous
    //     test's promise resolves *during* the next test and overwrites its
    //     state (surfacing as a stale 'bloom' hydrated by test 1).
    vi.resetModules()
    setActivePinia(createPinia())
    modeChangedHandlers.length = 0
    stateHandlers.length = 0
    rpcConnected = true
    rpcCall.mockClear()
  })

  afterEach(() => {
    modeChangedHandlers.length = 0
    stateHandlers.length = 0
    rpcConnected = true
  })

  it('hydrates from mode.get on init and subscribes to session.event.mode.changed', async () => {
    const { useActiveMode } = await import('./useActiveMode')
    const { activeMode, initActiveMode, disposeActiveMode } = useActiveMode()
    initActiveMode()
    // allow the mode.get promise to resolve
    await Promise.resolve()
    await new Promise((r) => setTimeout(r, 0))
    expect(rpcCall).toHaveBeenCalledWith('mode.get', {})
    expect(activeMode.value?.kind).toBe('bloom')
    expect(modeChangedHandlers.length).toBe(1)
    disposeActiveMode()
  })

  it('updates the shared ref immediately when a mode.changed frame arrives', async () => {
    const { useActiveMode } = await import('./useActiveMode')
    const { activeMode, initActiveMode, disposeActiveMode } = useActiveMode()
    initActiveMode()
    expect(modeChangedHandlers.length).toBe(1)
    // Simulate the core gateway broadcast: { type, payload: { from, to } }
    modeChangedHandlers[0]({ type: 'event', payload: { from: 'bloom', to: 'omni-transcend' } })
    expect(activeMode.value?.kind).toBe('omni-transcend')
    expect(activeMode.value?.mode).toBe('omni-transcend')
    disposeActiveMode()
  })

  it('switchActiveMode applies the mode only after mode.switch confirms', async () => {
    const { useActiveMode, switchActiveMode } = await import('./useActiveMode')
    const { activeMode, initActiveMode, disposeActiveMode } = useActiveMode()
    initActiveMode()
    const result = await switchActiveMode('plan')
    // T05: no longer optimistic — the badge moves only once the gateway acks.
    expect(activeMode.value?.kind).toBe('plan')
    expect(rpcCall).toHaveBeenCalledWith('mode.switch', { kind: 'plan' })
    expect(result?.kind).toBe('plan')
    disposeActiveMode()
  })

  it('disposeActiveMode unsubscribes the frame handler', async () => {
    const { useActiveMode } = await import('./useActiveMode')
    const { initActiveMode, disposeActiveMode } = useActiveMode()
    initActiveMode()
    expect(modeChangedHandlers.length).toBe(1)
    disposeActiveMode()
    expect(modeChangedHandlers.length).toBe(0)
  })

  it('waits for the transport before hydrating (regression: mode.get never sent)', async () => {
    // The real bug: App.vue calls initActiveMode() from onMounted, while the
    // socket is still 'disconnected'. The old implementation fired mode.get
    // immediately, the call failed, `.catch(() => undefined)` hid it, and
    // activeMode stayed null forever -> the badge rendered `--unknown` and the
    // mode hint bar showed "Initializing · The gateway is loading…".
    // Measured: ZERO mode.get frames ever left the browser.
    rpcConnected = false
    vi.resetModules()
    const { useActiveMode } = await import('./useActiveMode')
    const { activeMode, initActiveMode, disposeActiveMode } = useActiveMode()
    initActiveMode()

    // Not connected yet -> must NOT have asked the gateway.
    expect(rpcCall).not.toHaveBeenCalledWith('mode.get', {})

    // The socket connects -> now it hydrates.
    stateHandlers.forEach((h) => h('connected'))
    await Promise.resolve()
    await new Promise((r) => setTimeout(r, 0))
    expect(rpcCall).toHaveBeenCalledWith('mode.get', {})
    expect(activeMode.value?.kind).toBe('bloom')
    disposeActiveMode()
  })
})
