import { ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

/**
 * F1「离开会话页面任务不停止」的前端桥：切回会话时读取后端增量持久化的
 * partial_text，供 UI 恢复「正在生成」的消息占位（而非丢失流式文本）。
 * 后端 context.status RPC 已暴露 partial_text（gateway → context::session::context_status）。
 */
export interface ContextStatusResponse {
  session?: string
  used_tokens?: number
  budget?: number
  ratio?: number
  alert?: string
  entries?: number
  summary?: string | null
  partial_text?: string | null
  partialText?: string | null
}

export function usePartialTextRestore() {
  const rpc = useRpcStore()
  const partialText = ref<string | null>(null)
  const loading = ref(false)
  const error = ref('')

  async function load(sessionKey: string): Promise<string | null> {
    loading.value = true
    error.value = ''
    try {
      const res = await rpc.call<ContextStatusResponse>('context.status', { sessionKey })
      const p = res?.partial_text ?? res?.partialText ?? null
      partialText.value = p && p.length > 0 ? p : null
      return partialText.value
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      partialText.value = null
      return null
    } finally {
      loading.value = false
    }
  }

  return { partialText, loading, error, load }
}
