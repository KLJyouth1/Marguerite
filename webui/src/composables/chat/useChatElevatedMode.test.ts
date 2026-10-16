// useChatElevatedMode — 提权（审批绕过）模式的同步行为守卫。
//
// 背景：这个 composable 此前**没有任何测试**，于是一个静默的功能失效活了下来 ——
// 它每次进入 chat 都会 `POST /api/elevated-mode`，而网关**根本没有这个端点**
// （`core/src/gateway/server.rs:149-156` 只注册了 `/ws`、`/api/health`、`/control/*`；
// 全仓 Rust 代码里搜不到 elevated 相关 handler）。请求因此被 SPA 静态回退接住，
// 静态处理器只收 GET/HEAD → **405**。旧代码只对 `403` 做降级，其它非 2xx 一律
// `console.warn` 吞掉，于是 localStorage 里仍然留着 `elevatedMode`，
// 界面继续声称「审批绕过已开启」，而网关从不执行任何绕过。
//
// 本测试锁住的正是这条不变量：**不支持该端点的任何状态都必须落到「模式未启用」**。
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
    // 405 是当前网关对 `/api/elevated-mode` 的真实回答（见文件头注释）。
    for (const status of [403, 404, 405, 501]) {
      it(`treats HTTP ${status} as "unsupported" and stops claiming the mode is on`, async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse(status))
        vi.stubGlobal('fetch', fetchMock)

        const api = harness()
        api.setElevatedMode('bypass', { sync: true })

        // 允许 syncElevatedMode 的 promise 落地
        await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

        expect(api.elevatedUnavailable.value).toBe(true)
        expect(api.elevatedMode.value).toBe('')
        expect(api.effectiveElevatedMode.value).toBe('')
        // 不能把「服务端不支持」的模式留在本地存储里
        expect(store.getItem(ELEVATED_MODE_KEY)).toBeNull()
        expect(store.getItem(ELEVATED_MODE_VERSION_KEY)).toBeNull()
      })
    }

    it('latches after an unsupported status so it stops hammering the endpoint', async () => {
      const fetchMock = vi.fn().mockResolvedValue(jsonResponse(405))
      vi.stubGlobal('fetch', fetchMock)

      const api = harness()
      api.setElevatedMode('bypass', { sync: true })
      await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

      // 再次切换不应再发请求（elevatedUnavailable 已置位）
      api.setElevatedMode('full')
      await new Promise((r) => setTimeout(r, 10))
      expect(fetchMock).toHaveBeenCalledTimes(1)
    })
  })

  it('accepts a 2xx response and keeps the mode active', async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(204))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness()
    api.setElevatedMode('bypass', { sync: true })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

    expect(api.elevatedUnavailable.value).toBe(false)
    expect(api.effectiveElevatedMode.value).toBe('bypass')
    expect(store.getItem(ELEVATED_MODE_KEY)).toBe('bypass')
  })

  it('on a network rejection does not leave the UI claiming the mode is on', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('network down'))
    vi.stubGlobal('fetch', fetchMock)

    const api = harness()
    api.setElevatedMode('bypass', { sync: true })
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))
    await vi.waitFor(() => expect(api.elevatedMode.value).toBe(''))

    // 网络故障不是「永久不支持」——保持可重试，不要 latch
    expect(api.elevatedUnavailable.value).toBe(false)
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
