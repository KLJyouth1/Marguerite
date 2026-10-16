/**
 * 3D 世界 ↔ 工作台双向桥接（agentBinding.ts 注释指定的「发送时机/连接降级」载体）。
 *
 * 方向一（游戏 → Agent）：拾取道具 → agentPromptFor(ctx) → outbox 排队 →
 *   连接就绪后逐条 chat.send（带 clientRequestId/clientMessageId 幂等去重）。
 *   断线不丢：outbox 常驻内存，reconnect 后 flush。
 *   sessionKey 来源：优先显式传入，否则尝试 `session.list` 取首个活跃会话；
 *   两者都拿不到 → outbox 保留并诚实上报 pending，**绝不向不存在的会话发送**。
 *
 * 方向二（Agent → 游戏）：ISLAND_TRACKED_EVENTS 逐事件订阅 → scenePulseFor
 *   → 场景脉冲指令（雾开/灯塔点亮/极光增强……），渲染层只消费指令。
 *
 * 诚实性红线（agentBinding.ts 文件头）：
 * - 不使用 supportsMethod() 门控（撞 rpc-contract 守卫），一律 try/catch 降级；
 * - 拾取提示词是确定性模板（agentPromptFor），不是「模型真的写了委托」；
 * - 未连接时 UI 须显示 pending 计数，不假装已送达。
 */

import { computed, onScopeDispose, ref, shallowRef, watch, type Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import { createClientMessageId, createClientRequestId } from '@/utils/chat/messageIdentity'
import {
  agentPromptFor,
  scenePulseFor,
  type PickupContext,
  type ScenePulse,
} from './agentBinding'
import type { PropKind, PropInteraction } from './propsSpec'
import type { World3dTier } from './tierProfile'
import { ISLAND_TRACKED_EVENTS } from '../islandRuntime'

/** outbox 中的一条待发委托 */
export interface PendingDispatch {
  /** 本地幂等 id（= clientMessageId，重试不变） */
  id: string
  prompt: string
  kind: PropKind
  createdAt: number
}

/** 道具拾取 → Agent 的入参（渲染层 PickupEvent 的桥接形态） */
export interface PickupDispatchInput {
  kind: PropKind
  interaction: PropInteraction
  name: string
}

interface ChatSendResult {
  message_id?: string
  task_id?: string
  sessionKey?: string
}

export interface UseWorld3dBridge {
  /** Agent→游戏：最近的场景脉冲（渲染层 watch 消费；无事件 = null） */
  lastPulse: Ref<ScenePulse | null>
  /** 游戏→Agent：待发委托队列（未连接时积压，UI 诚实显示） */
  outbox: Ref<PendingDispatch[]>
  /** 已成功送达计数 */
  deliveredCount: Ref<number>
  /** 发送失败计数（重试中） */
  failedCount: Ref<number>
  /** 当前派发会话键（null = 尚未解析，outbox 不发送） */
  sessionKey: Ref<string | null>
  /** 连接状态（UI 显示「已连接/离线」） */
  connected: Ref<boolean>
  /** 拾取道具 → 入队并尝试立即派发 */
  dispatchPickup: (ctx: PickupDispatchInput, tier: World3dTier) => void
  /** 手动 flush outbox（连接恢复后由 UI 触发或自动） */
  flush: () => Promise<void>
}

export function useWorld3dBridge(options: { sessionKey?: () => string | null } = {}): UseWorld3dBridge {
  const rpc = useRpcStore()

  const lastPulse = shallowRef<ScenePulse | null>(null)
  const outbox = shallowRef<PendingDispatch[]>([])
  const deliveredCount = ref(0)
  const failedCount = ref(0)
  const sessionKey = ref<string | null>(options.sessionKey?.() ?? null)
  const connected = computed(() => rpc.isConnected)

  // ───────── 方向二：Agent → 游戏（事件 → 场景脉冲） ─────────

  const disposers = ISLAND_TRACKED_EVENTS.map((event) =>
    rpc.on(event, () => {
      // tier 由消费方（Vue 容器）通过 applyPulse 前注入；这里用保守 'efficient' 推导强度，
      // 容器可用当前档位覆盖 scenePulseFor 的 profileOverride（见 scene.ts applyPulse）。
      const pulse = scenePulseFor(event, 'efficient')
      if (pulse) lastPulse.value = pulse
    }),
  )

  // ───────── 方向一：游戏 → Agent（拾取 → chat.send） ─────────

  /** 解析 sessionKey：显式来源优先，否则 session.list 取首个活跃会话。 */
  async function resolveSessionKey(): Promise<string | null> {
    const explicit = options.sessionKey?.() ?? sessionKey.value
    if (explicit) return explicit
    try {
      const list = await rpc.call<{ sessions?: Array<{ sessionKey?: string; session_key?: string; key?: string }> }>(
        'session.list',
        {},
      )
      const first = list?.sessions?.[0]
      const key = first?.sessionKey ?? first?.session_key ?? first?.key ?? null
      sessionKey.value = key
      return key
    } catch {
      // 未连接 / 方法不存在 → 保持 null，outbox 继续积压（不伪造会话）
      return null
    }
  }

  function dispatchPickup(ctx: PickupDispatchInput, tier: World3dTier): void {
    const pickup: PickupContext = { kind: ctx.kind, interaction: ctx.interaction, name: ctx.name, tier }
    const prompt = agentPromptFor(pickup)
    const id = createClientMessageId()
    outbox.value = [...outbox.value, { id, prompt, kind: ctx.kind, createdAt: Date.now() }]
    // 尽力立即派发（未连接则留在 outbox）
    void flush()
  }

  async function flush(): Promise<void> {
    if (outbox.value.length === 0) return
    const key = await resolveSessionKey()
    if (!key || !rpc.isConnected) return // 诚实：拿不到会话就不发
    const pending = [...outbox.value]
    outbox.value = []
    for (const item of pending) {
      try {
        await rpc.call<ChatSendResult>('chat.send', {
          message: item.prompt,
          sessionKey: key,
          clientRequestId: createClientRequestId(),
          clientMessageId: item.id,
          _source: 'island-world3d',
        })
        deliveredCount.value += 1
      } catch {
        failedCount.value += 1
        // 失败回队（保持顺序），下次 flush 重试
        outbox.value = [...outbox.value, item]
      }
    }
  }

  // 连接恢复 → 自动 flush（不丢拾取委托）
  const stopWatch = watch(connected, (isOn) => {
    if (isOn) void flush()
  })

  onScopeDispose(() => {
    for (const off of disposers) off()
    stopWatch()
  })

  return {
    lastPulse,
    outbox,
    deliveredCount,
    failedCount,
    sessionKey,
    connected,
    dispatchPickup,
    flush,
  }
}
