// 网关 readiness 判据测试（2026-10-22）。
//
// 锁两个**用户真实撞到的**缺陷：
//   1. `onboarding.status` 用 10s + reconnect 时，撞上最长 30s 的 Windows 能力 canary
//      → 超时→断连→重连→再超时 → 顶栏**永久「连接中」**；
//   2. 读取失败（status 仍为 null）时旧实现直接 return 'ready' —— **假就绪**，
//      用户看到绿色却发不出第一句话。

import { describe, expect, it } from 'vitest'
import {
  OPTIONAL_SESSION_RPC_TIMEOUT_MS,
  optionalSessionRpcCallOptions,
  readinessRpcCallOptions,
  sandboxSetupRpcCallOptions,
} from '@/composables/chat/sessionBootstrapAdmission'
import { readinessNeedsAction } from '@/composables/setup/useReadinessSummary'

describe('readinessRpcCallOptions —— 不许继承 optionalSession 的短超时', () => {
  it('超时必须覆盖 30s 的 canary 探针（否则必然超时）', () => {
    expect(readinessRpcCallOptions.timeoutMs).toBeGreaterThan(30_000)
  })

  it('超时/中断都不得砸掉共享 WS（reject，不是 reconnect）', () => {
    // reconnect 会把所有 in-flight 请求 reject 成 Disconnected，
    // 并触发整条 WS 重连 —— 而重连后同样排在 canary 后面，形成自维持循环。
    expect(readinessRpcCallOptions.timeoutAction).toBe('reject')
    expect(readinessRpcCallOptions.abortAction).toBe('reject')
  })

  it('与 sandboxSetup 的宽限保持一致（同一类诊断读取，同一判据）', () => {
    expect(readinessRpcCallOptions.timeoutMs).toBe(sandboxSetupRpcCallOptions.timeoutMs)
    expect(readinessRpcCallOptions.timeoutAction).toBe(sandboxSetupRpcCallOptions.timeoutAction)
  })

  it('optionalSession 仍保留其"真卡住就重连"语义（不被本次改动波及）', () => {
    // chat 关键路径确实需要这个语义，改它会引入别的问题。
    expect(optionalSessionRpcCallOptions.timeoutMs).toBe(OPTIONAL_SESSION_RPC_TIMEOUT_MS)
    expect(optionalSessionRpcCallOptions.timeoutAction).toBe('reconnect')
  })
})

describe('readinessNeedsAction —— 只据实判定，不无端"需配置"', () => {
  it('llm 就绪时不得报需要动作（真实 onboarding.status 形态）', () => {
    // 这是 2026-10-22 从运行中的网关抓到的真实 payload 形状：
    //   needsOnboarding=false, llmSource='config',
    //   sectionDetails.llm = { status:'complete', blocking:false }
    expect(
      readinessNeedsAction({
        needsOnboarding: false,
        llmSource: 'config',
        sectionDetails: {
          llm: { status: 'complete', blocking: false, actionRequired: false },
        },
      }),
    ).toBe(false)
  })

  it('真正缺配置时才报（blocking / missing / degraded）', () => {
    expect(readinessNeedsAction({ needsOnboarding: true })).toBe(true)
    expect(readinessNeedsAction({ llmSource: 'missing_env' })).toBe(true)
    expect(
      readinessNeedsAction({ sectionDetails: { llm: { status: 'missing' } } }),
    ).toBe(true)
    expect(
      readinessNeedsAction({ sectionDetails: { llm: { status: 'degraded' } } }),
    ).toBe(true)
  })

  it('null 状态不算"需要动作"（无证据不下断言）', () => {
    // 注意：这不代表 UI 该显示 ready —— readiness 计算里对 status=null
    // 单独返回 'connecting'，二者职责不同、互补。
    expect(readinessNeedsAction(null)).toBe(false)
  })
})
