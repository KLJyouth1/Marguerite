import { ref, watch, type Ref } from 'vue'
import {
  type EscalationAckResult,
  type EscalationDecision,
  type EscalationEvent,
  type EscalationTrigger,
} from '@/types/escalation'
import { payloadBelongsToSession } from '@/utils/chat/plans'

type EscalationRpcClient = {
  call: <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>
  on: (event: string, handler: (...args: unknown[]) => void) => () => void
}

const TRIGGERS = new Set<EscalationTrigger>([
  'high_impact_side_effect',
  'intent_ambiguity',
  'timeout',
  'budget_exceeded',
])

/** 从任意来源值解析结构化 escalation 事件（core `EscalationEvent` wire 形状）。
 *  非法/缺失 id 返回 null → 忽略。 */
export function normalizeEscalation(value: unknown): EscalationEvent | null {
  if (!value || typeof value !== 'object') return null
  let source = value as Record<string, unknown>
  // 事件信封可能只有 kind + escalation 嵌套字段
  if (source.escalation && typeof source.escalation === 'object') {
    source = source.escalation as Record<string, unknown>
  }
  const id = source.id
  if (!id || typeof id !== 'string' || !id) return null
  const trigger = source.trigger as EscalationTrigger | undefined
  if (!trigger || !TRIGGERS.has(trigger)) return null
  return {
    id,
    session: String(source.session || ''),
    trigger,
    severity: Number(source.severity) || 0,
    reason: String(source.reason || ''),
    context: source.context && typeof source.context === 'object'
      ? source.context as Record<string, unknown>
      : {},
    suggested_action: String(source.suggested_action || source.suggestedAction || ''),
    created_at: String(source.created_at || source.createdAt || ''),
  }
}

export interface UseChatEscalationsOptions {
  rpc: EscalationRpcClient
  sessionKey: Ref<string>
  notifyError?: (message: string) => void
}

/**
 * M2-C4：跨端 escalation「需要确认」事件流。
 *
 * - 订阅 `session.event.escalation`（事件总线 route = `{session}.escalation`），
 *   同一时刻仅展示一条 `pending`，其余入 `queue` 排队。
 * - 用户在确认卡选择 继续/中止 → `escalation.ack { id, decision }`
 *   （core `escalation_registry` 全局裁决桥，任一端都能闭环）→ 弹出下一队列项。
 * - 订阅时一次性拉取 `escalation.pending` 恢复早于 socket 连接的升级事件。
 */
export function useChatEscalations(options: UseChatEscalationsOptions) {
  const { rpc, sessionKey, notifyError } = options
  const pending = ref<EscalationEvent | null>(null)
  const queue = ref<EscalationEvent[]>([])
  const busy = ref(false)
  const error = ref('')
  const resolvedCount = ref(0)

  function belongs(payload: unknown): boolean {
    return payloadBelongsToSession(payload, sessionKey.value)
  }

  function enqueue(event: EscalationEvent) {
    if (!event.id) return
    if (pending.value && pending.value.id === event.id) return
    if (queue.value.some(item => item.id === event.id)) return
    if (pending.value === null) {
      pending.value = event
    } else {
      queue.value = [...queue.value, event]
    }
  }

  function applyEscalationEnvelope(value: unknown) {
    if (!belongs(value)) return
    const event = normalizeEscalation(value)
    if (!event) return
    // core EscalationEvent 携带 session 字段；非当前会话的升级事件忽略
    if (event.session && sessionKey.value && event.session !== sessionKey.value) return
    enqueue(event)
  }

  async function fetchPending() {
    try {
      const result = await rpc.call<{ escles?: unknown } | { esclesations?: unknown } | { escalations?: unknown[]; ok?: boolean }>(
        'escalation.pending',
      )
      const list = (result as { escalations?: unknown[] })?.escalations || []
      for (const raw of list) {
        const event = normalizeEscalation(raw)
        if (event) enqueue(event)
      }
    } catch {
      // pending 拉取是兜底恢复，失败不阻塞事件流
    }
  }

  async function resolve(event: EscalationEvent, decision: EscalationDecision) {
    const current = pending.value
    if (busy.value || !current || current.id !== event.id) return
    busy.value = true
    error.value = ''
    try {
      const result = await rpc.call<EscalationAckResult>('escalation.ack', {
        id: event.id,
        decision,
      })
      if (result?.ok === false) {
        error.value = result.error || 'escalation 决策失败'
        return
      }
      resolvedCount.value += 1
      pending.value = queue.value[0] ?? null
      queue.value = queue.value.slice(1)
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      notifyError?.(error.value)
    } finally {
      busy.value = false
    }
  }

  function subscribe(): () => void {
    const unsubs = [
      rpc.on('session.event.escalation', applyEscalationEnvelope),
      rpc.on('escalation', applyEscalationEnvelope),
    ]
    void fetchPending()
    return () => unsubs.forEach(unsubscribe => unsubscribe())
  }

  watch(sessionKey, () => {
    pending.value = null
    queue.value = []
    busy.value = false
    error.value = ''
  }, { immediate: true })

  return {
    pending,
    queue,
    busy,
    error,
    resolvedCount,
    resolve,
    subscribe,
  }
}
