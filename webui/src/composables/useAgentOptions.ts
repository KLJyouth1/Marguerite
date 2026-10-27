import { ref } from 'vue'
import {
  waitForSessionRpcConnection,
} from '@/composables/chat/sessionBootstrapAdmission'
import { useRpcStore } from '@/stores/rpc'
import i18n from '@/i18n'
import { normalizeAgentId } from '@/utils/chat/sessionKeys'
import type { AgentOption, AgentsListResponse } from '@/types/rpc'
import type { RpcCallOptions } from '@/lib/rpc'

/**
 * 默认会话（`agent:main:*`）在 `agents.list` 尚未返回时的兜底身份。
 *
 * 名字走 i18n：此前硬编码英文 `Main Agent`，于是中文界在冷启动 / 列表失败时
 * 侧边栏会永久显示 "Main Agent"（`chat.mainAgent` 是六语言齐备的既有键）。
 */
function mainAgentFallback(): AgentOption {
  return { id: 'main', name: i18n.global.t('chat.mainAgent') }
}

// Module-level singleton state so sidebar session metadata shares one agents
// list and one fetch. Mirrors the singleton pattern in useConfirm.
const agents = ref<AgentOption[]>([])
const agentListError = ref(false)
// Dedupes concurrent loadAgents() calls onto a single in-flight request; cleared
// once it settles so a later call can refresh.
let loadPromise: Promise<void> | null = null

/**
 * Shared `agents.list` fetch for sidebar session metadata. IDs are normalized
 * once here so every sidebar consumer sees the same canonical value.
 */
export function useAgentOptions(readCallOptions?: RpcCallOptions) {
  const rpc = useRpcStore()

  function loadAgents(): Promise<void> {
    if (loadPromise) return loadPromise
    loadPromise = (async () => {
      agentListError.value = false
      try {
        await waitForSessionRpcConnection(rpc, readCallOptions)
        const data = readCallOptions
          ? await rpc.call<AgentsListResponse>(
              'agents.list',
              undefined,
              readCallOptions,
            )
          : await rpc.call<AgentsListResponse>('agents.list')
        agents.value = (data?.agents || [])
          .map(a => ({
            id: normalizeAgentId(a.id || a.agentId || a.name || ''),
            name: a.name || a.id || a.agentId || 'Agent',
            model: a.model || '',
          }))
          .filter((a: AgentOption) => !!a.id)
      } catch (err: unknown) {
        console.warn('[useAgentOptions] agents.list error:', err instanceof Error ? err.message : err)
        agentListError.value = true
        if (!agents.value.length) agents.value = [mainAgentFallback()]
      } finally {
        loadPromise = null
      }
    })()
    return loadPromise
  }

  return { agents, agentListError, loadAgents }
}
