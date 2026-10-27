import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRpcStore } from '@/stores/rpc'

export interface LlmProfile {
  model?: string
  base_url?: string
  profileId?: string
  label?: string
}

export type LlmProfiles = Record<string, LlmProfile>

/** 单条档案行（`config.get.llm_profile_rows`，按档案 id 列，不与 provider 归并）。 */
export interface LlmProfileRow {
  id: string
  provider: string
  model: string
  base_url?: string
  label?: string
  /** cloud / custom / local —— 输入框模型选择器分三档的唯一依据 */
  kind?: string
  wire_api?: string
  builtin?: boolean
  active?: boolean
}

interface ConfigSnapshot {
  llm?: {
    provider?: string
    model?: string
    base_url?: string
  }
  llm_profiles?: LlmProfiles
  llm_profile_rows?: LlmProfileRow[]
}

/**
 * Models.json 的前端只读投影。
 *
 * `llm_profiles` 是网关公开的稳定字段；这里负责归一化缺失/脏数据、事件防抖
 * 和卸载解绑，避免聊天页、设置页各自复制一份解析规则。
 */
export function useLlmProfiles() {
  const rpc = useRpcStore()
  const profiles = ref<LlmProfiles>({})
  const rows = ref<LlmProfileRow[]>([])
  const loading = ref(false)
  const error = ref<unknown>(null)
  const active = ref({ provider: '', model: '', baseUrl: '' })
  let timer: ReturnType<typeof setTimeout> | null = null
  let unsub: (() => void) | null = null

  const profileList = computed(() => Object.entries(profiles.value).map(([provider, profile]) => ({
    provider,
    ...profile,
  })))
  const profileCount = computed(() => profileList.value.length)

  async function load() {
    if (rpc.state !== 'connected') return
    loading.value = true
    error.value = null
    try {
      const cfg = await rpc.call<ConfigSnapshot>('config.get')
      const raw = cfg?.llm_profiles
      profiles.value = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {}
      // 档案级清单：`llm_profiles` 以 provider 为 key 会互相覆盖，
      // 只有这个数组能完整列出同一 provider 的多份档案（多个本地模型）。
      const rawRows = cfg?.llm_profile_rows
      rows.value = Array.isArray(rawRows)
        ? rawRows.filter((r): r is LlmProfileRow => Boolean(r) && typeof r === 'object' && Boolean(r.id))
        : []
      active.value = {
        provider: String(cfg?.llm?.provider ?? ''),
        model: String(cfg?.llm?.model ?? ''),
        baseUrl: String(cfg?.llm?.base_url ?? ''),
      }
    } catch (cause) {
      error.value = cause
      profiles.value = {}
      rows.value = []
    } finally {
      loading.value = false
    }
  }

  function scheduleReload() {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      void load()
    }, 200)
  }

  onMounted(() => {
    void load()
    unsub = rpc.on('models.changed', scheduleReload)
  })

  // 挂载瞬间往往还在握手（冷启动），`load()` 会因为未连接直接返回 —— 之后
  // 再没有任何触发点，模型选择器就永远停在「—」，用户以为"每次都要重新选"。
  // 连接就绪时补一次加载。
  watch(
    () => rpc.state,
    (next, prev) => {
      if (next === 'connected' && prev !== 'connected') void load()
    },
  )

  onUnmounted(() => {
    if (timer) clearTimeout(timer)
    unsub?.()
  })

  return {
    profiles,
    rows,
    profileList,
    profileCount,
    active,
    loading,
    error,
    load,
  }
}
