import { ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

interface McpToolEntry {
  name?: string
  server?: string
}

// 模块级单例：整个应用共享一份 MCP 工具名单。首个消费者拉取一次，
// 后续所有 RunTrace 实例直接复用，避免每次渲染都重复请求。
const mcpNames = ref<Set<string>>(new Set())
const serverByTool = ref<Map<string, string>>(new Map())
let inflight: Promise<void> | null = null
let loaded = false

/**
 * 工具来源标注：识别工具调用是否来自 MCP 服务器（如宝塔面板 btpanel-mcp），
 * 供聊天工具调用卡片展示来源徽标。
 *
 * 数据源是 `mcp.tools.list` 而**不是** `tools.catalog`——后者只覆盖本地工具，
 * 其中的 MCP 条目恒为空，导致来源徽标永远不显示。调用时带 `cachedOnly: true`：
 * 渲染路径只读后端缓存，绝不触发 MCP 探测（那是网络/子进程副作用），真实探测由
 * 「插件与技能 › 工具与能力」面板显式发起。
 *
 * 组件测试等无 active Pinia 的环境安全降级（不抛错）。
 */
export function useToolSources() {
  let rpc: ReturnType<typeof useRpcStore> | null = null
  try {
    rpc = useRpcStore()
  } catch {
    rpc = null
  }

  async function ensureLoaded(): Promise<void> {
    if (!rpc || loaded) return
    if (inflight) return inflight
    inflight = (async () => {
      try {
        const res = await rpc!.call<{ tools?: McpToolEntry[] }>('mcp.tools.list', {
          cachedOnly: true,
        })
        const names = new Set<string>()
        const servers = new Map<string, string>()
        for (const item of res?.tools ?? []) {
          if (!item?.name) continue
          names.add(item.name)
          if (item.server) servers.set(item.name, item.server)
        }
        mcpNames.value = names
        serverByTool.value = servers
        // 只有拿到真实响应才算加载完成；未探测过（空清单）同样是有效结果，
        // 不该每次渲染都重问一遍。
        loaded = true
      } catch {
        // 目录加载失败时保持空集，聊天渲染不受影响（下次渲染可重试）
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

/** 单测用：重置模块级单例状态（否则用例之间会共享 `loaded`）。 */
export function __resetToolSourcesForTest(): void {
  mcpNames.value = new Set()
  serverByTool.value = new Map()
  inflight = null
  loaded = false
}

/**
 * 外部**显式探测**成功后调用：丢弃「已加载」标记并重新拉取缓存。
 *
 * 没有它会出现一个真实的时序缺口：渲染路径先以 `cachedOnly` 拿到空清单并标记已加载，
 * 用户随后在面板里点了「探测」，服务器连上了，但徽标仍不亮——要刷新页面才生效。
 */
export async function refreshToolSources(): Promise<void> {
  loaded = false
  inflight = null
  await useToolSources().ensureLoaded()
}
