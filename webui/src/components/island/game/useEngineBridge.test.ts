/**
 * W5.6：`cache.status` → EngineSnapshot 的**诚实**映射。
 *
 * 锁的契约：
 *   1. 服务端前缀覆盖（token 占比）、合格请求命中率（请求占比）、本地命中率
 *      **三个字段分开**上报，绝不 `Math.max(...)` 合成一个数；
 *   2. 新字段有两种拼写（计划名 / 网关现役 server_ 前缀名）都要识别；
 *   3. 只有旧 `server_cache_hit_rate` 时回退为 coverage 并显式标 `legacy`，
 *      请求级命中率保持 null（不猜）；
 *   4. 全空时三个值都是 null、来源也是 null。
 */
import { effectScope, type EffectScope } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

const rpcCall = vi.fn()

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({
    isConnected: true,
    call: rpcCall,
    on: () => () => {},
  }),
}))

import { useEngineBridge } from './useEngineBridge'

function respondWith(cache: Record<string, unknown> | null) {
  rpcCall.mockImplementation((method: string) => {
    if (method === 'cache.status') return Promise.resolve(cache)
    // null → safeCall 视作「方法不存在」，hasGameRpc=false
    if (method === 'game.status') return Promise.resolve(null)
    return Promise.resolve({})
  })
}

let scope: EffectScope | null = null

afterEach(() => {
  scope?.stop()
  scope = null
  rpcCall.mockReset()
})

async function refresh() {
  scope = effectScope()
  const bridge = scope.run(() => useEngineBridge())!
  await bridge.refresh()
  return bridge.snapshot.value
}

describe('useEngineBridge · honest cache metrics (W5.6)', () => {
  it('splits coverage, eligible-request hit rate and client rate (plan field names)', async () => {
    respondWith({
      cache_coverage_pct: 42,
      request_hit_rate_pct: 80,
      client_hit_rate: 90,
      server_cache_hit_rate: 42, // 旧别名同时在场时不得改变口径
    })
    const snap = await refresh()

    expect(snap.cacheCoveragePct).toBe(42)
    expect(snap.cacheRequestHitRatePct).toBe(80)
    expect(snap.clientHitRatePct).toBe(90)
    expect(snap.cacheMetricsSource).toBe('live')

    // 旧实现 Math.max(client, server_cache_hit_rate) 会得到 90 —— 那是混算。
    expect(snap.cacheCoveragePct).not.toBe(Math.max(42, 90))
    // 合成字段已删除，避免任何消费者再把三个数混成一个。
    expect('cacheHitRate' in snap).toBe(false)
  })

  it('accepts the gateway server_-prefixed field names', async () => {
    respondWith({
      server_cache_coverage_pct: 55,
      server_cache_request_hit_rate_pct: 25,
      client_hit_rate: 90,
      server_cache_hit_rate: 55,
    })
    const snap = await refresh()

    expect(snap.cacheCoveragePct).toBe(55)
    expect(snap.cacheRequestHitRatePct).toBe(25)
    expect(snap.clientHitRatePct).toBe(90)
    expect(snap.cacheMetricsSource).toBe('live')
    // 不能退化成 max(55, 90) = 90。
    expect(snap.cacheCoveragePct).not.toBe(90)
  })

  it('falls back to server_cache_hit_rate with an explicit legacy marker only', async () => {
    respondWith({ server_cache_hit_rate: 33, client_hit_rate: 77 })
    const snap = await refresh()

    expect(snap.cacheMetricsSource).toBe('legacy')
    expect(snap.cacheCoveragePct).toBe(33)
    // 请求级指标不可得 —— 必须 null，而不是拿旧口径冒充。
    expect(snap.cacheRequestHitRatePct).toBeNull()
    expect(snap.clientHitRatePct).toBe(77)
    // 旧实现会取 max(33, 77) = 77。
    expect(snap.cacheCoveragePct).not.toBe(77)
  })

  it('ignores non-finite values instead of fabricating numbers', async () => {
    respondWith({
      cache_coverage_pct: Number.NaN,
      request_hit_rate_pct: Number.POSITIVE_INFINITY,
      server_cache_hit_rate: 12,
      client_hit_rate: 5,
    })
    const snap = await refresh()

    expect(snap.cacheMetricsSource).toBe('legacy')
    expect(snap.cacheCoveragePct).toBe(12)
    expect(snap.cacheRequestHitRatePct).toBeNull()
    expect(snap.clientHitRatePct).toBe(5)
  })

  it('keeps all three metrics null when the gateway reports nothing', async () => {
    respondWith({})
    const snap = await refresh()

    expect(snap.cacheCoveragePct).toBeNull()
    expect(snap.cacheRequestHitRatePct).toBeNull()
    expect(snap.clientHitRatePct).toBeNull()
    expect(snap.cacheMetricsSource).toBeNull()
  })

  it('handles an unavailable cache.status RPC without inventing metrics', async () => {
    respondWith(null)
    const snap = await refresh()

    expect(snap.cacheCoveragePct).toBeNull()
    expect(snap.cacheRequestHitRatePct).toBeNull()
    expect(snap.clientHitRatePct).toBeNull()
    expect(snap.cacheMetricsSource).toBeNull()
  })
})
