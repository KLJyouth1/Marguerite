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
/** `model.tier.get` 的返回（core/gateway/methods.rs）。 */
interface TierPayload {
  locked_tier?: string | null
  thinking_effort?: string
  effective_tier?: string
  budget?: {
    remaining_ratio?: number
    state?: string
  }
}
/** `cache.status` 的返回；命中率/覆盖率都是 0..100 的百分数（不是 0..1）。 */
interface CacheStatusPayload {
  /** W5.1 计划字段名（coverage = cache_read / 归一后总输入） */
  cache_coverage_pct?: number
  /** W5.1 计划字段名（合格请求中至少命中一次的比例） */
  request_hit_rate_pct?: number
  /** 网关现役字段名（W5.1 实现带 `server_` 前缀；两个拼写都接受，避免版本漂移） */
  server_cache_coverage_pct?: number
  server_cache_request_hit_rate_pct?: number
  /** 本地前缀缓存命中率（独立量纲） */
  client_hit_rate?: number
  /** 旧口径别名（= coverage）。只有新字段缺席时才用，并显式标 legacy。 */
  server_cache_hit_rate?: number
}

const EMPTY: EngineSnapshot = {
  connected: false,
  modeKind: null,
  reasoningEffort: null,
  maxSwarmShards: null,
  hasTierRpc: false,
  hasBudgetRpc: false,
  hasGameRpc: false,
  effectiveTier: null,
  tierLocked: null,
  budgetState: null,
  budgetRemaining: null,
  cacheCoveragePct: null,
  cacheRequestHitRatePct: null,
  clientHitRatePct: null,
  cacheMetricsSource: null,
}

export interface UseEngineBridge {
  snapshot: Ref<EngineSnapshot>
  refresh: () => Promise<void>
  /** 上次刷新的人类可读状态（用于面板诚实标注） */
  lastError: Ref<string | null>
}

/** 取第一个有限数值；null/undefined/NaN/Infinity 一律跳过（不编造数字）。 */
function firstFinite(...values: Array<number | undefined>): number | null {
  for (const value of values) {
    if (typeof value === 'number' && Number.isFinite(value)) return value
  }
  return null
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

    const [mode, spec, tier, cache] = await Promise.all([
      safeCall<ModeGetPayload>('mode.get'),
      safeCall<ModeSpecPayload>('mode.spec'),
      safeCall<TierPayload>('model.tier.get'),
      safeCall<CacheStatusPayload>('cache.status'),
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

    // 真实三档分层（core 已落地 model.tier.get）：档位 / 锁定 / 预算台账。
    // 方法不存在时 tier=null → 保持 false 与 null，UI 诚实显示「未知」。
    if (tier) {
      next.hasTierRpc = true
      next.effectiveTier = typeof tier.effective_tier === 'string' ? tier.effective_tier : null
      next.tierLocked = typeof tier.locked_tier === 'string' ? tier.locked_tier : null
      next.budgetState = typeof tier.budget?.state === 'string' ? tier.budget.state : null
      next.budgetRemaining =
        typeof tier.budget?.remaining_ratio === 'number' && Number.isFinite(tier.budget.remaining_ratio)
          ? tier.budget.remaining_ratio
          : null
      next.hasBudgetRpc = !!(tier.budget && typeof tier.budget === 'object')
      // 档位决策给出的思考强度优先于 mode.spec 的模式默认值（前者是本次生效值）。
      if (typeof tier.thinking_effort === 'string' && tier.thinking_effort) {
        next.reasoningEffort = tier.thinking_effort
      }
    }

    // W5.6：把「服务端前缀覆盖」与「合格请求命中率」**分开**上报。
    // 旧实现 `Math.max(client, server)` 把两个不同量纲（本地 lookups vs 上游 token）
    // 合成一个数 —— 任一为零/偏低时它都会虚报，用户无法据此判断缓存是否真的在复用。
    const coverage = firstFinite(
      cache?.cache_coverage_pct,
      cache?.server_cache_coverage_pct,
    )
    const requestHitRate = firstFinite(
      cache?.request_hit_rate_pct,
      cache?.server_cache_request_hit_rate_pct,
    )
    // 旧网关只有 `server_cache_hit_rate`（W5.1 已把它定义为 coverage 的别名）。
    // 回退时必须显式标 legacy —— 它**不是** request-level 命中率。
    const legacyCoverage = firstFinite(cache?.server_cache_hit_rate)
    next.clientHitRatePct = firstFinite(cache?.client_hit_rate)
    next.cacheCoveragePct = coverage ?? legacyCoverage
    next.cacheRequestHitRatePct = requestHitRate
    next.cacheMetricsSource =
      coverage !== null
        ? 'live'
        : legacyCoverage !== null
          ? 'legacy'
          : requestHitRate !== null
            ? 'live'
            : null

    // 尚无 game.status（保留探测，存在后即可点亮游戏态字段）。
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
