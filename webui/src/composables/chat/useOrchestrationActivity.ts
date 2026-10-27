// P2·E 编排活动日志：订阅 `agent.started / agent.stopped` 与 `orchestrate*` 事件，
// 维护一个最多 N 条、新的在前的滚动列表，供 ChatOrchestrationStatus 实时展示。
//
// 订阅面与 ChatSwarmPopover 自带的事件订阅各自独立（那个只在面板打开时订阅），
// 本 composable 常驻但只对编排类事件记账，其余事件零开销。
import { ref, type Ref } from 'vue'

export interface OrchestrationActivityItem {
  /** 稳定 id（ts + 序号，避免重复 key）。 */
  id: string
  kind: 'started' | 'stopped' | 'event'
  /** 原始事件名（agent.started / agent.stopped / …）。 */
  event: string
  /** 后端 payload：form / form_label / goal / ok 等，供组件翻译展示。 */
  payload?: Record<string, unknown>
  ts: string
}

export type OrchestrationRpcClient = {
  /** 与 rpc store 的 on 签名一致：通配订阅的 handler 收到全部参数（event + payload）。 */
  on(event: string, handler: (...args: unknown[]) => void): () => void
}

export interface OrchestrationActivity {
  items: Ref<OrchestrationActivityItem[]>
  /** 订阅编排类事件，返回退订函数（供父级统一挂进 unsubs）。 */
  subscribe: () => () => void
  unsubscribe: () => void
  clear: () => void
}

const MAX_ITEMS = 7

export function useOrchestrationActivity(
  rpc: OrchestrationRpcClient,
): OrchestrationActivity {
  const items = ref<OrchestrationActivityItem[]>([])
  let unsubs: Array<() => void> = []
  let seq = 0

  function push(
    kind: OrchestrationActivityItem['kind'],
    event: string,
    payload?: Record<string, unknown>,
  ) {
    seq += 1
    const ts = new Date().toLocaleTimeString()
    items.value = [
      { id: `${Date.now()}-${seq}`, kind, event, payload, ts },
      ...items.value,
    ].slice(0, MAX_ITEMS)
  }

  function subscribe() {
    if (unsubs.length > 0) return () => unsubscribe()
    unsubs.push(
      rpc.on('*', (...args: unknown[]) => {
        const rawEvent = typeof args[0] === 'string' ? args[0] : ''
        const rawPayload = args[1]
        // 精确命中 agent.*（后端 orchestrate.run 发布 agent.started/stopped）；
        // `orchestrate` 命中未来编排相关命名（兼容性兜底），避免重复记账。
        if (rawEvent.startsWith('session.event.')) return
        const payloadObj =
          rawPayload && typeof rawPayload === 'object'
            ? (rawPayload as Record<string, unknown>)
            : undefined
        if (rawEvent === 'agent.started') {
          push('started', rawEvent, payloadObj)
          return
        }
        if (rawEvent === 'agent.stopped') {
          push('stopped', rawEvent, payloadObj)
          return
        }
        if (rawEvent.includes('agent.') || rawEvent.includes('orchestrate')) {
          push('event', rawEvent, payloadObj)
        }
      }),
    )
    // 与 Vue 生态一致：subscribe() 返回退订函数，便于父级统一挂进 unsubs 列表。
    return () => unsubscribe()
  }

  function unsubscribe() {
    for (const off of unsubs) off()
    unsubs = []
  }

  function clear() {
    items.value = []
  }

  return {
    items,
    subscribe,
    unsubscribe,
    clear,
  }
}