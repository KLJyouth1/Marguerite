import { ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

/**
 * modlens 视觉取证：图片(base64) → 结构化 JSON 文档（文字/阅读顺序/语义/版面）。
 * 后端 gateway 的 modlens.analyze 已在 SUPPORTED_METHODS 注册；无视觉模型时
 * 降级为元数据兜底（source=metadata-fallback），绝不静默失败。
 */
export interface ModlensDocument {
  summary: string
  source: string
  reading_order?: string[]
  blocks?: unknown[]
  layout?: unknown
}

export function useModlens() {
  const rpc = useRpcStore()
  const busy = ref(false)
  const error = ref('')

  async function analyze(imageBase64: string, hint: string): Promise<ModlensDocument | null> {
    busy.value = true
    error.value = ''
    try {
      const res = await rpc.call<{ ok?: boolean; document?: ModlensDocument; error?: string }>(
        'modlens.analyze',
        { imageBase64, hint },
      )
      if (!res?.ok) {
        error.value = res?.error || '视觉分析失败'
        return null
      }
      return res.document ?? null
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      return null
    } finally {
      busy.value = false
    }
  }

  return { analyze, busy, error }
}
