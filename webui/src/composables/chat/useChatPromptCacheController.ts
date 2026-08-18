// Prompt Cache Keepalive 状态控制器（从 ChatView.vue 抽出的本地编排函数）。
// 仅读写共享状态中的 promptCacheKeepalive* 字段，并依赖 rpc 与两个派生可用性标记。
import { type ComputedRef } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type {
  PromptCacheKeepaliveStatus,
  PromptCacheKeepaliveStatusUpdate,
} from '@/types/promptCacheKeepalive'

export interface ChatPromptCacheControllerDeps {
  rpc: ReturnType<typeof useRpcStore>
  /** sessions.promptCacheKeepalive.* 两个方法是否都受支持。 */
  promptCacheKeepaliveAvailable: ComputedRef<boolean>
  /** 当前会话是否已准备好（无待处理的 session intent）。 */
  promptCacheKeepaliveSessionReady: ComputedRef<boolean>
}

export function useChatPromptCacheController(
  state: ChatViewState,
  deps: ChatPromptCacheControllerDeps,
) {
  const { sessionKey, promptCacheKeepaliveStatus } = state

  async function refreshPromptCacheKeepaliveStatus() {
    const key = sessionKey.value
    if (
      !key
      || !deps.promptCacheKeepaliveAvailable.value
      || !deps.promptCacheKeepaliveSessionReady.value
    ) return
    try {
      const next = await deps.rpc.call<PromptCacheKeepaliveStatus>(
        'sessions.promptCacheKeepalive.status',
        { key },
      )
      if (sessionKey.value === key) promptCacheKeepaliveStatus.value = next
    } catch {
      // The settings dialog owns actionable RPC errors. Menu refresh is best effort.
    }
  }

  function onPromptCacheKeepaliveSaved(update: PromptCacheKeepaliveStatusUpdate) {
    if (update.sessionKey === sessionKey.value) {
      promptCacheKeepaliveStatus.value = update.status
    }
  }

  return { refreshPromptCacheKeepaliveStatus, onPromptCacheKeepaliveSaved }
}
