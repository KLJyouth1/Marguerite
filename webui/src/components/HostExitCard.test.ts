// @vitest-environment happy-dom
//
// Regression coverage for the "宿主进程已退出" (host exited) card staying on
// screen while a healthy Python sidecar is running. Root cause: the card keyed
// its visibility off `exited` alone — a diagnostic RECORD — and ignored
// `sidecar_present`, the LIVE fact. After a transient exit (e.g. the sidecar-
// reuse path logging `复用的 Python 侧车进程已消失`) a fresh sidecar came up
// healthy, but the on-disk record survived, so the card rendered forever.
//
// The card must therefore be driven by BOTH signals. These tests mount the real
// `HostExitCard` with the real provided state and drive the raw payload, so the
// visibility binary is exercised end-to-end (composable computed → card).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import i18n from '@/i18n'

// The card reads its state through `useHostExitEvidence()`, whose only external
// dependency is the RPC store. Mock it so the composable builds without a live
// gateway; the tests drive the payload directly, so `call` is never awaited.
const rpcMock = vi.hoisted(() => ({
  isConnected: false,
  state: 'disconnected' as string,
  calls: [] as string[],
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    get isConnected() { return rpcMock.isConnected },
    get state() { return rpcMock.state },
    call: vi.fn(async (method: string) => {
      rpcMock.calls.push(method)
      return { exited: true, source: 'record', evidence: null, sidecar_present: false, sidecar_uptime_ms: null }
    }),
    waitForConnection: vi.fn(async () => undefined),
  }),
}))

import HostExitCard from './HostExitCard.vue'
import {
  provideHostExitEvidence,
  type HostExitEvidencePayload,
  type HostExitEvidenceState,
} from '@/composables/useHostExitEvidence'

const mountedApps: Array<{ app: App; el: HTMLElement }> = []

interface MountedCard {
  readonly el: HTMLElement
  state(): HostExitEvidenceState
  setPayload(payload: HostExitEvidencePayload): Promise<void>
}

/** A fully-spelled payload; override only the fields a case cares about. */
function payload(over: Partial<HostExitEvidencePayload>): HostExitEvidencePayload {
  return {
    exited: false,
    source: null,
    evidence: null,
    sidecar_present: false,
    sidecar_uptime_ms: null,
    ...over,
  }
}

/**
 * Mount the real card inside a harness that owns the shared exit-evidence
 * instance exactly like `App.vue` does, and hand back a payload setter.
 */
async function mountCard(): Promise<MountedCard> {
  let state!: HostExitEvidenceState
  const Harness = defineComponent({
    setup() {
      // Mirrors `App.vue`: the always-mounted root owns the single instance and
      // publishes it through provide/inject for the card to consume.
      state = provideHostExitEvidence()
      return () => h(HostExitCard)
    },
  })

  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(Harness)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  mountedApps.push({ app, el })

  return {
    el,
    state: () => state,
    setPayload: async (next) => {
      state.payload.value = next
      await nextTick()
    },
  }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

afterEach(() => {
  while (mountedApps.length) {
    const { app, el } = mountedApps.pop() as { app: App; el: HTMLElement }
    app.unmount()
    el.remove()
  }
})

describe('HostExitCard — visibility tracks the CURRENT sidecar, not a stale record', () => {
  it('stays hidden when an exit was recorded but a sidecar is present (the fix)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'record',
        evidence: { code: 0, uptime_ms: 3000, stderr_tail: '' },
        sidecar_present: true,
        sidecar_uptime_ms: 1200,
      }),
    )

    // Both raw signals are true — the card must still hide, because a healthy
    // sidecar is registered right now.
    expect(card.state().hasEvidence.value).toBe(true)
    expect(card.state().sidecarPresent.value).toBe(true)
    expect(card.el.querySelector('.host-exit')).toBeNull()
  })

  it('NEVER shows a cross-restart historical record (startup false-alarm fix)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'record',
        evidence: { code: 0, uptime_ms: 3000, stderr_tail: '' },
        sidecar_present: false,
      }),
    )

    // 真机缺陷（2026-10-23）：落盘记录是上一次运行的历史，且 `sidecar_present`
    // 在网关侧不可靠（python 明明在跑也报 false）。历史记录**永不**上屏 ——
    // 它属于诊断页，不该做成启动横幅。
    expect(card.el.querySelector('.host-exit')).toBeNull()
    // 历史事实仍然可读（供诊断页复用），只是不驱动这张卡片。
    expect(card.state().hasEvidence.value).toBe(true)
    expect(card.state().source.value).toBe('record')
  })

  it('shows a CURRENT-run exit immediately (no settle delay for a real failure)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'memory',
        evidence: { code: 1, uptime_ms: 2000, stderr_tail: '' },
        sidecar_present: false,
      }),
    )

    expect(card.el.querySelector('.host-exit')).not.toBeNull()
    // 细横幅只放人话标题；退出码/存活时长收在「详情」里（旧实现把它们塞进
    // ErrorState 大卡片，真机上 400×180 常驻右下角遮挡主内容）。
    expect(card.el.querySelector('.host-exit__text')?.textContent).toContain('helper engine')
    expect(card.el.querySelector('.host-exit__panel')).toBeNull()
  })

  it('stays hidden when nothing has exited (normal, healthy state)', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({ exited: false, sidecar_present: true, sidecar_uptime_ms: 500 }),
    )

    expect(card.state().hasEvidence.value).toBe(false)
    expect(card.el.querySelector('.host-exit')).toBeNull()
  })

  it('closes on demand and re-arms only after the sidecar comes back', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'memory',
        evidence: { code: 1, uptime_ms: 2000, stderr_tail: 'boom' },
        sidecar_present: false,
      }),
    )

    // 详情默认折叠：一屏只给一句人话，诊断信息按需展开。
    expect(card.el.querySelector('.host-exit__panel')).toBeNull()
    card.el.querySelector<HTMLButtonElement>('[data-testid="host-exit-details"]')?.click()
    await nextTick()
    expect(card.el.querySelector('.host-exit__panel')?.textContent).toContain('boom')

    // 用户关闭后本次运行不再打扰。
    card.el.querySelector<HTMLButtonElement>('[data-testid="host-exit-dismiss"]')?.click()
    await nextTick()
    expect(card.el.querySelector('.host-exit')).toBeNull()

    // 侧车恢复 → 关闭状态复位（下一次真退出仍要能被看见）。
    await card.setPayload(
      payload({ exited: true, source: 'memory', evidence: null, sidecar_present: true }),
    )
    await card.setPayload(
      payload({
        exited: true,
        source: 'memory',
        evidence: { code: 2, uptime_ms: 3000, stderr_tail: '' },
        sidecar_present: false,
      }),
    )
    expect(card.el.querySelector('.host-exit')).not.toBeNull()
  })
})

describe('HostExitCard — retry actually restarts the host process', () => {
  afterEach(() => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__
  })

  it('calls sidecar_stop + sidecar_start instead of only re-reading evidence', async () => {
    const calls: string[] = []
    ;(window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
      core: {
        invoke: async (cmd: string) => {
          calls.push(cmd)
          if (cmd === 'sidecar_status') return { status: 'failed' }
          if (cmd === 'sidecar_stop') return null
          if (cmd === 'sidecar_start') return { status: 'running' }
          return null
        },
      },
    }

    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        source: 'memory',
        evidence: { code: null, uptime_ms: 56 * 60 * 1000, stderr_tail: 'boom' },
        sidecar_present: false,
      }),
    )

    const retry = card.el.querySelector<HTMLButtonElement>('[data-testid="host-exit-restart"]')
    expect(retry).not.toBeNull()
    retry?.click()
    // 让 restart() 里的 await 链跑完
    await new Promise((resolve) => setTimeout(resolve, 0))
    await nextTick()

    expect(calls).toContain('sidecar_status')
    expect(calls).toContain('sidecar_stop')
    expect(calls).toContain('sidecar_start')
  })
})

describe('HostExitCard — a stale record must not stick after the sidecar comes up', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    rpcMock.isConnected = true
    rpcMock.state = 'connected'
    rpcMock.calls.length = 0
  })

  afterEach(() => {
    vi.useRealTimers()
    rpcMock.isConnected = false
    rpcMock.state = 'disconnected'
  })

  it('re-reads evidence while visible and stops once the card hides', async () => {
    const card = await mountCard()
    await card.setPayload(
      payload({
        exited: true,
        // 用本次运行的真实退出：历史记录要等宽限窗口，轮询起点会随之后移，
        // 本用例验的是「可见即轮询、隐藏即停」这条逻辑，与来源无关。
        source: 'memory',
        evidence: { code: null, uptime_ms: 1000, stderr_tail: '' },
        sidecar_present: false,
      }),
    )

    // 可见 → 每 3s 复读一次（首轮由 onMounted 触发）。
    await vi.advanceTimersByTimeAsync(3000)
    const whileVisible = rpcMock.calls.filter(method => method === 'host.exit_evidence').length
    expect(whileVisible).toBeGreaterThanOrEqual(2)

    // 侧车已注册 → 卡片隐藏 → 轮询停止，不再发请求。
    await card.setPayload(
      payload({
        exited: true,
        source: 'record',
        evidence: { code: null, uptime_ms: 1000, stderr_tail: '' },
        sidecar_present: true,
      }),
    )
    const atHide = rpcMock.calls.filter(method => method === 'host.exit_evidence').length
    await vi.advanceTimersByTimeAsync(9000)
    const afterHide = rpcMock.calls.filter(method => method === 'host.exit_evidence').length
    expect(afterHide).toBe(atHide)
  })
})
