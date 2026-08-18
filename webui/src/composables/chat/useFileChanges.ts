import { ref } from 'vue'
import type { FileChangeListPayload, FileChangePayload } from '@/types/rpc'

type FileChangeRpc = {
  call: <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>
  supportsMethod?: (method: string) => boolean
  markMethodUnavailable?: (method: string) => void
}

export interface UseFileChangesOptions {
  rpc: FileChangeRpc
  limit?: number
}

/**
 * 读取 Agent 最近对工作区文件的真实改动（`workspace.file_changes.list`）。
 *
 * 与 artifacts 不同，文件更改是「近期全局台账」，无分页、无 session 维度；
 * 前端在「文件更改」面板打开时拉取一次即可。
 */
export function useFileChanges(options: UseFileChangesOptions) {
  const changes = ref<FileChangePayload[]>([])
  const loading = ref(false)
  const available = ref(false)

  async function load(): Promise<boolean> {
    if (options.rpc.supportsMethod && !options.rpc.supportsMethod('workspace.file_changes.list')) {
      available.value = false
      return false
    }
    loading.value = true
    try {
      const response = await options.rpc.call<FileChangeListPayload>('workspace.file_changes.list', {
        limit: options.limit ?? 50,
      })
      changes.value = Array.isArray(response?.items)
        ? response.items.filter(item => item && typeof item === 'object')
        : []
      available.value = true
      return true
    } catch {
      available.value = false
      return false
    } finally {
      loading.value = false
    }
  }

  function reset() {
    changes.value = []
    available.value = false
    loading.value = false
  }

  return {
    changes,
    loading,
    available,
    load,
    reset,
  }
}
