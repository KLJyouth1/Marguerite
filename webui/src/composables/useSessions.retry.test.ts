import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { effectScope } from 'vue'

const rpcCall = vi.fn()
const waitForConnection = vi.fn(async () => undefined)

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => ({ call: rpcCall, waitForConnection }),
}))

import { useSessions, SESSION_LIST_MISSING_TOLERANCE, SESSION_LIST_RETRY_DELAYS_MS, itemKey, timestampValue } from './useSessions'

// A first `sessions.list` read can fail while the gateway is still warming up
// (cold start after an install). The sidebar must heal itself instead of
// staying on "无法加载任务" until the user clicks reload — that stuck state is
// what shipped in the 0.2.6 console.
describe('useSessions — bounded self-healing after a failed list read', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    rpcCall.mockReset()
    waitForConnection.mockClear()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  function mountSessions() {
    const scope = effectScope()
    const api = scope.run(() => useSessions())
    if (!api) throw new Error('useSessions did not run inside the scope')
    return { scope, api }
  }

  it('retries with backoff until the list loads, then clears the error', async () => {
    rpcCall
      .mockRejectedValueOnce(new Error('gateway warming up'))
      .mockRejectedValueOnce(new Error('gateway warming up'))
      .mockResolvedValueOnce({
        sessions: [{ key: 'agent:main:webchat:healed', title: 'Healed chat' }],
      })
    const { api } = mountSessions()

    await api.loadSessions()
    expect(rpcCall).toHaveBeenCalledTimes(1)
    expect(api.sessionListError.value).toBe(true)

    await vi.advanceTimersByTimeAsync(SESSION_LIST_RETRY_DELAYS_MS[0])
    expect(rpcCall).toHaveBeenCalledTimes(2)
    expect(api.sessionListError.value).toBe(true)

    await vi.advanceTimersByTimeAsync(SESSION_LIST_RETRY_DELAYS_MS[1])
    expect(rpcCall).toHaveBeenCalledTimes(3)
    expect(api.sessionListError.value).toBe(false)
    expect(api.sessionsList.value).toHaveLength(1)
  })

  it('stops after the bounded budget instead of retrying forever', async () => {
    rpcCall.mockRejectedValue(new Error('still down'))
    const { api } = mountSessions()

    await api.loadSessions()
    for (const delay of SESSION_LIST_RETRY_DELAYS_MS) {
      await vi.advanceTimersByTimeAsync(delay)
    }
    await vi.advanceTimersByTimeAsync(60_000)

    expect(rpcCall).toHaveBeenCalledTimes(1 + SESSION_LIST_RETRY_DELAYS_MS.length)
    expect(api.sessionListError.value).toBe(true)
  })

  it('cancels a pending retry when the owning scope is disposed', async () => {
    rpcCall.mockRejectedValue(new Error('still down'))
    const { scope, api } = mountSessions()

    await api.loadSessions()
    scope.stop()
    await vi.advanceTimersByTimeAsync(60_000)

    expect(rpcCall).toHaveBeenCalledTimes(1)
  })
})

// 刷新/重连返回一个更短或降级的列表时，侧栏不得把用户已经看得到的会话「变少」。
// `loadSessions` 因此以 key 为锚做 merge：响应里有的行以服务端为准，响应里这次没
// 有的本地行不立刻删，只有连续 SESSION_LIST_MISSING_TOLERANCE 次成功刷新都缺失
// 时才移除（重连/读错误恢复时重置计数）。
describe('useSessions — merge (not wholesale replace) on refresh', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    rpcCall.mockReset()
    waitForConnection.mockClear()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  function mountSessions() {
    const scope = effectScope()
    const api = scope.run(() => useSessions())
    if (!api) throw new Error('useSessions did not run inside the scope')
    return { scope, api }
  }

  it('keeps locally-known rows when a refresh returns a shorter list', async () => {
    rpcCall
      .mockResolvedValueOnce({
        sessions: [
          { key: 'a', title: 'A' },
          { key: 'b', title: 'B' },
          { key: 'c', title: 'C' },
        ],
      })
      .mockResolvedValueOnce({ sessions: [{ key: 'b', title: 'B updated' }] })
    const { api } = mountSessions()

    await api.loadSessions()
    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b', 'c'])

    await api.loadSessions()
    // a & c are missing once → retained; b is taken from the server (authoritative).
    expect(api.sessionsList.value.map(itemKey)).toEqual(['b', 'a', 'c'])
    const b = api.sessionsList.value.find(entry => itemKey(entry) === 'b')
    expect((b as { title?: string }).title).toBe('B updated')
  })

  it('drops a row only after N consecutive successful misses', async () => {
    rpcCall.mockResolvedValueOnce({
      sessions: [{ key: 'a', title: 'A' }, { key: 'b', title: 'B' }],
    })
    for (let i = 0; i < SESSION_LIST_MISSING_TOLERANCE; i += 1) {
      rpcCall.mockResolvedValueOnce({ sessions: [{ key: 'a', title: 'A' }] })
    }
    const { api } = mountSessions()

    await api.loadSessions()
    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b'])

    // The first miss keeps the row; only the Nth miss drops it.
    for (let i = 1; i <= SESSION_LIST_MISSING_TOLERANCE; i += 1) {
      await api.loadSessions()
      const expected = i < SESSION_LIST_MISSING_TOLERANCE ? ['a', 'b'] : ['a']
      expect(api.sessionsList.value.map(itemKey)).toEqual(expected)
    }
  })

  it('resets miss counters after a recovered read error (reconnect boundary)', async () => {
    rpcCall
      .mockResolvedValueOnce({ sessions: [{ key: 'a' }, { key: 'b' }] })
      .mockResolvedValueOnce({ sessions: [{ key: 'a' }] }) // b miss #1
      .mockRejectedValueOnce(new Error('socket dropped')) // read error → keep list
      .mockResolvedValueOnce({ sessions: [{ key: 'a' }] }) // after recovery: b miss #1 again
    const { api } = mountSessions()

    await api.loadSessions()
    await api.loadSessions()
    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b'])

    await api.loadSessions()
    expect(api.sessionListError.value).toBe(true)
    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b'])

    await api.loadSessions()
    // Counters were reset on the recovered read, so b survives this miss too.
    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b'])
  })

  it('warns (does not stay silent) when patching a session absent from the local list', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    rpcCall.mockResolvedValueOnce({ sessions: [{ key: 'a', title: 'A' }] })
    const { api } = mountSessions()

    await api.loadSessions()

    expect(api.applySessionRunStatus({ session_key: 'ghost', run_status: 'running' })).toBe(false)
    expect(warn).toHaveBeenCalledWith(
      expect.stringContaining('no local row'),
      'ghost',
      'event=sessions.changed',
    )
  })
})

// 侧栏曾出现同一会话重复多行（实测：标题重复的「只回复两个字：收到」×2、「收到」×3）。
// 来源是两个引擎/分页合并后的 `sessions.list` 自身带重复 key —— 前端只要原样渲染
// 就会重复，且 :key 冲突让 Vue 复用错行。响应内部的重复必须在这里收敛。
describe('useSessions — duplicates inside one sessions.list response', () => {
  beforeEach(() => {
    rpcCall.mockReset()
    waitForConnection.mockClear()
  })

  function mountSessions() {
    const scope = effectScope()
    const api = scope.run(() => useSessions())
    if (!api) throw new Error('useSessions did not run inside the scope')
    return { scope, api }
  }

  it('collapses duplicate keys, keeping the last authoritative row', async () => {
    rpcCall.mockResolvedValueOnce({
      sessions: [
        { key: 'a', title: 'A first' },
        { key: 'b', title: 'B' },
        { key: 'a', title: 'A second' },
      ],
    })
    const { api } = mountSessions()

    await api.loadSessions()

    expect(api.sessionsList.value.map(itemKey)).toEqual(['a', 'b'])
    const a = api.sessionsList.value.find(entry => itemKey(entry) === 'a')
    expect((a as { title?: string }).title).toBe('A second')
    expect(api.allSessions.value).toHaveLength(2)
  })
})

// 时间戳字段曾经只认毫秒整数：桥接层给 RFC3339 字符串时 `Number('2026-…')` 是
// NaN → 静默落到 0，会话在侧栏排到最后并显示成 1970。两种形态都必须归一成毫秒。
describe('useSessions — timestamp normalization', () => {
  beforeEach(() => {
    rpcCall.mockReset()
    waitForConnection.mockClear()
  })

  it('accepts milliseconds, numeric strings and RFC3339 alike', () => {
    expect(timestampValue(1_700_000_000_000)).toBe(1_700_000_000_000)
    expect(timestampValue('1700000000000')).toBe(1_700_000_000_000)
    expect(timestampValue('2026-01-02T03:04:05Z')).toBe(Date.parse('2026-01-02T03:04:05Z'))
    expect(timestampValue('')).toBeNull()
    expect(timestampValue(null)).toBeNull()
    expect(timestampValue('not-a-date')).toBeNull()
  })

  it('sorts sessions by an RFC3339 updated_at instead of collapsing to 0', async () => {
    rpcCall.mockResolvedValueOnce({
      sessions: [
        { key: 'old', title: 'Old', updated_at: '2026-01-01T00:00:00Z' },
        { key: 'new', title: 'New', updated_at: '2026-01-02T00:00:00Z' },
      ],
    })
    const scope = effectScope()
    const api = scope.run(() => useSessions())
    if (!api) throw new Error('useSessions did not run inside the scope')

    await api.loadSessions()

    expect(api.allSessions.value.map(item => item.key)).toEqual(['new', 'old'])
  })
})