// Routing candidate models — Pinia store（**不是** composable 里的模块级单例）。
//
// 与 `activeMode` 同理：`webui/` 是路由懒加载 + 多 chunk 打包，同一个模块会被
// 复制进多个 chunk。本地模型中心（`LocalModelsView`）与设置中心（`useSetupCatalog`）
// 分属不同 chunk，若用模块级 ref，两边各持一份、部署后必然各显示各的。
// 候选真源统一由 `config.get` 的 `model_routing.candidateModels`（Rust 兜底输出
// 同时给 camel/snake 两种形态）驱动：
//   * `applyConfig(cfg)` —— 复用调用方已经取回的 `config.get` 结果，避免重复 RPC；
//   * `refresh()` —— 主动重新 `config.get` 拉取（部署/激活成功后的即时对账）；
//   * `subscribe()` —— 被动订阅 `models.changed` 事件，路由变更时自动刷新。
import { ref } from 'vue'
import { defineStore } from 'pinia'
import { useRpcStore } from '@/stores/rpc'

/**
 * 从一条 `config.get` 载荷中提取候选模型 ID 列表。
 * 后端 Rust 兜底（core/src/gateway/config.rs）同时输出 camel/snake 两种形态，
 * 这里两个键都读；Python 侧车在线时走 `config.get` 原文，也兼容两种键。
 */
export function extractCandidateModels(cfg: unknown): string[] {
  if (!cfg || typeof cfg !== 'object') return []
  const c = cfg as Record<string, unknown>
  const routing = (c.model_routing ?? c.modelRouting) as Record<string, unknown> | undefined
  if (!routing || typeof routing !== 'object') return []
  const list = routing.candidateModels ?? routing.candidate_models
  if (!Array.isArray(list)) return []
  return list.filter((id): id is string => typeof id === 'string')
}

export const useCandidateModelsStore = defineStore('candidateModels', () => {
  /** 路由候选 profile ID 列表（与 `config.get` 的 `candidateModels` 同一真源）。 */
  const ids = ref<string[]>([])
  /** 是否已从后端成功读过至少一次（区分「还没拉」与「拉了但为空」）。 */
  const loaded = ref(false)
  /** 最近一次拉取失败信息；空串表示无错误。 */
  const error = ref('')

  /** 用已取回的 `config.get` 载荷写入候选（不额外发 RPC）。 */
  function applyConfig(cfg: unknown): void {
    ids.value = extractCandidateModels(cfg)
    loaded.value = true
  }

  /** 主动 `config.get` 重读候选真源。 */
  async function refresh(): Promise<void> {
    const rpc = useRpcStore()
    try {
      const cfg = await rpc.call<unknown>('config.get')
      applyConfig(cfg)
      error.value = ''
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e)
    }
  }

  /**
   * 订阅 `models.changed` 事件做被动刷新。返回 unsubscribe；挂在
   * onMounted / onUnmounted 上与组件生命周期对齐。可在多个调用方各订阅一份，
   * 事件幂等，重复刷新不会写坏状态。
   */
  function subscribe(): () => void {
    const rpc = useRpcStore()
    let active = true
    const unsubscribe = rpc.on('models.changed', () => {
      if (active) void refresh()
    })
    return () => {
      active = false
      unsubscribe()
    }
  }

  return { ids, loaded, error, applyConfig, refresh, subscribe }
})