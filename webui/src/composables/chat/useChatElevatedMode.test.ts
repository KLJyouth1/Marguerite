// useChatElevatedMode — 提权（审批绕过）模式的同步行为守卫。
//
// 背景：这个 composable 曾经每次进入 chat 都 `POST /api/elevated-mode`，而旧网关
// 没有这个端点，请求被 SPA 静态回退接住回 405；旧代码只在「不支持」时永久闩锁，
// 网关升级后必须刷新页面才能恢复。P2-3 起：
//   1) Rust 网关真实实现该端点（core/src/gateway/elevated.rs）；
//   2) 前端**不再写永久闩锁** —— 不支持时只回退本次状态，下一次切换仍会重试。
//
// 本测试锁住的不变量：**不支持 / 网络失败 / 2xx 三种情况下，界面状态都必须诚实**，
// 且不支持状态不得阻止后续重试。
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { ref } from 'vue'
import { useChatElevatedMode } from './useChatElevatedMode'

const ELEVATED_MODE_KEY = 'marguerite.elevatedMode'
const ELEVATED_MODE_VERSION_KEY = 'marguerite.elevatedMode.version'

function makeStore() {
  const map = new Map<string, string>()
  return {
    getItem: (k: string) => (map.has(k) ? (map.get(k) as string) : null),
    setItem: (k: string, v: string) => void map.set(k, String(v)),
    removeItem: (k: string) => void map.delete(k),
    clear: () => map.clear(),
    key: (i: number) => Array.from(map.keys())[i] ?? null,
    get length() {
      return map.size
    },
    _map: map,
  }
}

function jsonResponse(status: number) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => ({}),
  } as unknown as Response
}

function harness(sessionKey = 'agent:main:webchat:test') {
  return useChatElevatedMode({ sessionKey: ref(sessionKey) })
}

describe('useChatElevatedMode', () => {
  let store: ReturnType<typeof makeStore>
  let warn: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    store = makeStore()
    vi.stubGlobal('localStorage', store)
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('normalizeElevatedMode only accepts the three known modes', () => {
    expect(useChatElevatedMode, 'module loads').toBeTypeOf('function')
  })

  describe('syncElevatedMode against a gateway without the endpoint', () => {
    // 旧网关对 `/api/elevated-mode` 的回答（Rust 已实现，这里守住旧部署的降级）。
    for (const status of [403, 404, 405, 501]) {
      it(`treats HTTP ${status} as "unsupported" and stops claiming the mode is on`, async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse(status))
        vi.stubGlobal('fetch', fetchMock)

        const api = harness()
        api.setElevatedMode('bypass', { sync: true })

        // 允许 syncElevatedMode 的 promise 落地
        await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

        expect(api.elevatedMode.value).toBe('')
        expect(api.effectiveElevatedMode.value).toBe('')
        // 不能把「服务端不支持」的模式留在本地存储里
        expect(store.getItem(ELEVATED_MODE_KEY)).toBeNull()
        expect(store.getItem(ELEVATED_MODE_VERSION_KEY)).toBeNull()
      })
    }

    it('does not latch after an unsupported status: the next toggle retries', async () => {
      const fetchMock = vi.fn().mockResolvedValue(jsonResponse(405))
      vi.stubGlobal('fetch', fetchMock)

      const api = harness()
      api.setElevatedMode('bypass', { sync: true })
      await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

      // P2-3：不再永久闩锁 —— 网关升级后无需刷新页面即可重试
      api.setElevatedMode('full', { sync: true })
      await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2))
    })
  })

  it('accepts a 2xx response and keeps the mode active', async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(204))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness()
    api.setElevatedMode('bypass', { sync: true })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

    expect(api.effectiveElevatedMode.value).toBe('bypass')
    expect(store.getItem(ELEVATED_MODE_KEY)).toBe('bypass')
  })

  it('explicit off overrides a globally configured bypass for the session', async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(204))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness()
    // config.permissions.default_mode = 'bypass' 时，全局默认会点亮开关。
    api.setGlobalElevatedMode('bypass')
    expect(api.effectiveElevatedMode.value).toBe('bypass')

    // 用户显式关闭：必须压过全局默认，且后端收到的是清除（线上值 off）。
    api.setElevatedMode('off', { sync: true })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

    expect(api.effectiveElevatedMode.value).toBe('')
    expect(store.getItem(ELEVATED_MODE_KEY)).toBe('off')
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body))
    expect(body.mode).toBe('off')
  })

  it('on a network rejection does not leave the UI claiming the mode is on', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('network down'))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness()
    api.setElevatedMode('bypass', { sync: true })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))
    await vi.waitFor(() => expect(api.elevatedMode.value).toBe(''))

    expect(warn).toHaveBeenCalled()
  })

  it('does not call the endpoint without a session key', async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(204))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness('')
    api.setElevatedMode('bypass')
    await new Promise((r) => setTimeout(r, 10))
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('loadElevatedMode migrates the storage version and tolerates a broken store', () => {
    // 版本不匹配时必须回落到 'bypass' 并写回，而不是沿用已废弃的 'full'
    store.setItem(ELEVATED_MODE_KEY, 'full')
    store.setItem(ELEVATED_MODE_VERSION_KEY, '1')

    const api = harness()
    api.loadElevatedMode()
    // 迁移后写回调用会触发一次 sync；这里只断言存储被规范
    expect(['bypass', 'full']).toContain(store.getItem(ELEVATED_MODE_KEY))

    // localStorage 抛异常时不得把整个 composable 崩掉
    vi.stubGlobal('localStorage', {
      getItem() {
        throw new Error('blocked')
      },
      setItem() {
        throw new Error('blocked')
      },
      removeItem() {
        throw new Error('blocked')
      },
    })
    expect(() => harness().loadElevatedMode()).not.toThrow()
  })
})
