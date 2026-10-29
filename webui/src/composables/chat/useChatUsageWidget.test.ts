import { ref } from 'vue'
import { describe, expect, it, vi } from 'vitest'

import { useChatUsageWidget } from './useChatUsageWidget'
import type { RpcCallOptions } from '@/lib/rpc'

describe('useChatUsageWidget background reads', () => {
  it('uses the injected bounded options without changing its public loader', async () => {
    const readCallOptions: RpcCallOptions = {
      timeoutMs: 2_000,
      timeoutAction: 'reconnect',
      abortAction: 'reconnect',
    }
    const rpc = {
      waitForConnection: vi.fn().mockResolvedValue(undefined),
      call: vi.fn().mockResolvedValue({
        sessions: [{
          sessionKey: 'agent:main:webchat:usage',
          inputTokens: 12,
          outputTokens: 8,
        }],
      }),
    }
    const api = useChatUsageWidget({
      rpc,
      readCallOptions,
      sessionKey: ref('agent:main:webchat:usage'),
      tokenVizEnabled: () => false,
    })

    await api.loadCurrentSessionUsage()

    expect(rpc.waitForConnection).toHaveBeenCalledWith(
      2_000,
      undefined,
      {
        timeoutAction: 'reconnect',
        abortAction: 'reconnect',
      },
    )
    expect(rpc.call).toHaveBeenCalledWith(
      'usage.status',
      { sessionKey: 'agent:main:webchat:usage' },
      readCallOptions,
    )
    expect(api.usageAccum.value).toMatchObject({ input: 12, output: 8 })
  })

  // 回归：页脚 token 会「倒流」。实时累加已经到 111k，`usage.status` 轮询拿回台账里的
  // 旧值（102）时，旧实现直接赋值把数字改小，用户看到 111552→102→再涨。
  // 两个来源都是本会话的**累计量**，合并必须逐字段取大。
  it('merges backend totals monotonically so polling never rewinds live counts', async () => {
    const rpc = {
      waitForConnection: vi.fn().mockResolvedValue(undefined),
      call: vi.fn().mockResolvedValue({
        sessions: [{
          sessionKey: 'agent:main:webchat:usage',
          input_tokens: 100,
          output_tokens: 2,
          cache_read_tokens: 40,
        }],
      }),
    }
    const api = useChatUsageWidget({
      rpc,
      sessionKey: ref('agent:main:webchat:usage'),
      tokenVizEnabled: () => false,
    })
    api.usageAccum.value.input = 111_000
    api.usageAccum.value.output = 552
    api.usageAccum.value.cacheRead = 44_000

    await api.loadCurrentSessionUsage()

    expect(api.usageAccum.value.input).toBe(111_000)
    expect(api.usageAccum.value.output).toBe(552)
    expect(api.usageAccum.value.cacheRead).toBe(44_000)
  })
})
