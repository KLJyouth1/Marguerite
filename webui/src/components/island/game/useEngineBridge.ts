/**
 * 引擎状态桥接 —— 把真实网关 RPC 归一成游戏可消费的 EngineSnapshot。
 *
 * 诚实性（蓝图 §9 / 任务红线）：
 * - 一切字段来自**既有 RPC**：`mode.get`（当前模式）、`mode.spec`（BloomSpec：
 *   max_swarm_shards / reasoning_effort / thinking_depth）。
 * - core 分片正在新增 tier/budget/game RPC：本桥对「方法不存在 / 连接未就绪」一律
 *   try/catch 降级为 null/false，**绝不编造引擎数值**，UI 据此显示「未连接 / 未知」。
 * - 不使用 supportsMethod() 门控未声明方法（避免撞 rpc-contract 守卫），而是直接调用 +
 *   失败容错；已声明的 mode.get / mode.spec 必然可达。
 */

import { onScopeDispose, ref, shallowRef, type Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import { normalizeModeKind } from '@/stores/activeMode'
import type { EngineSnapshot } from './gameLogic'

interface ModeSpecBloom {
  max_swarm_shards?: number
  reasoning_effort?: string
  thinking_depth?: number
}
interface ModeSpecPayload {
  kind?: string
  bloom?: ModeSpecBloom | null
}
interface ModeGetPayload {
  kind?: string
  mode?: string
}

const EMPTY: EngineSnapshot = {
  connected: false,
  modeKind: null,
  reasoningEffort: null,
  maxSwarmShards: null,
  hasTierRpc: false,
  hasBudgetRpc: false,
  hasGameRpc: false,
}

export interface UseEngineBridge {
  snapshot: Ref<EngineSnapshot>
  refresh: () => Promise<void>
  /** 上次刷新的人类可读状态（用于面板诚实标注） */
  lastError: Ref<string | null>
}

export function useEngineBridge(): UseEngineBridge {
  const rpc = useRpcStore()
  const snapshot = shallowRef<EngineSnapshot>({ ...EMPTY })
  const lastError = ref<string | null>(null)

  async function safeCall<T>(method: string): Promise<T | null> {
    try {
      return await rpc.call<T>(method, {})
    } catch {
      // 方法不存在 / 未连接 / 超时 —— 一律视为「该字段未知」，不抛出到 UI。
      return null
    }
  }

  async function refresh(): Promise<void> {
    if (!rpc.isConnected) {
      try {
        await rpc.waitForConnection(3000)
      } catch {
        /* 仍未连接 → 保持 EMPTY，UI 诚实标灰 */
      }
    }
    const next: EngineSnapshot = { ...EMPTY, connected: rpc.isConnected }
    if (!next.connected) {
      snapshot.value = next
      lastError.value = 'gateway offline'
      return
    }

    const [mode, spec] = await Promise.all([
      safeCall<ModeGetPayload>('mode.get'),
      safeCall<ModeSpecPayload>('mode.spec'),
    ])
    next.modeKind = mode?.kind ? normalizeModeKind(mode.kind) : null
    const bloom = spec?.bloom
    if (bloom) {
      next.reasoningEffort = typeof bloom.reasoning_effort === 'string' ? bloom.reasoning_effort : null
      next.maxSwarmShards =
        typeof bloom.max_swarm_shards === 'number' && Number.isFinite(bloom.max_swarm_shards)
          ? bloom.max_swarm_shards
          : null
    }

    // core 分片正在新增的 RPC：探测一次，存在才点亮对应符文。不存在 = false（不崩）。
    next.hasTierRpc = (await safeCall<unknown>('tier.get')) !== null
    next.hasBudgetRpc = (await safeCall<unknown>('budget.get')) !== null
    next.hasGameRpc = (await safeCall<unknown>('game.status')) !== null

    snapshot.value = next
    lastError.value = null
  }

  // 模式切换广播 → 即时更新符文（不等下次轮询）。
  const off = rpc.on('session.event.mode.changed', (payload: unknown) => {
    const kind = (payload as { kind?: string } | undefined)?.kind
    if (kind) {
      snapshot.value = { ...snapshot.value, modeKind: normalizeModeKind(kind) }
    }
  })

  onScopeDispose(() => off())

  return { snapshot, refresh, lastError }
}
