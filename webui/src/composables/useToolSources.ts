import { ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

interface CatalogTool {
  name: string
  description: string
  source?: string
  server?: string
}

// 模块级单例：整个应用共享一份 MCP 工具名单。首个消费者拉取一次，
// 后续所有 RunTrace 实例直接复用，避免每次渲染都重复请求 tools.catalog。
const mcpNames = ref<Set<string>>(new Set())
const serverByTool = ref<Map<string, string>>(new Map())
let inflight: Promise<void> | null = null

/**
 * 工具来源标注：识别工具调用是否来自 MCP 服务器（如宝塔面板 btpanel-mcp），
 * 供聊天工具调用卡片展示来源徽标。目录不可用时保持空集，不阻塞渲染。
 * 组件测试等无 active Pinia 的环境同样安全降级（不抛错）。
 */
export function useToolSources() {
  let rpc: ReturnType<typeof useRpcStore> | null = null
  try {
    rpc = useRpcStore()
  } catch {
    rpc = null
  }

  async function ensureLoaded(): Promise<void> {
    if (!rpc || mcpNames.value.size > 0) return
    if (inflight) return inflight
    inflight = (async () => {
      try {
        const res = await rpc!.call<{ items?: CatalogTool[] }>('tools.catalog')
        const names = new Set<string>()
        const servers = new Map<string, string>()
        for (const item of res?.items ?? []) {
          if (item.source === 'mcp') {
            names.add(item.name)
            if (item.server) servers.set(item.name, item.server)
          }
        }
        mcpNames.value = names
        serverByTool.value = servers
      } catch {
        // 目录加载失败时保持空集，聊天渲染不受影响
      } finally {
        inflight = null
      }
    })()
    return inflight
  }

  /** 返回 'mcp'（来自 MCP 服务器）、'builtin'（内置）或 null（未知/未加载）。 */
  function toolSource(name: string): 'mcp' | 'builtin' | null {
    return mcpNames.value.has(name) ? 'mcp' : null
  }

  /** MCP 服务器展示名；内置/未知工具返回空串。 */
  function toolServer(name: string): string {
    return serverByTool.value.get(name) || ''
  }

  return { ensureLoaded, toolSource, toolServer }
}
