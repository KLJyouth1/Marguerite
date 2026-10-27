/**
 * 团队看板状态层 —— `team.members` 契约的唯一消费点。
 *
 * 设计纪律：
 * - **门控**：先 `supportsMethod('team.members')`，网关没声明该方法就完全不发请求，
 *   视图渲染诚实空态（不编造成员）。
 * - **轮询**：5s 一次，只在视图挂载期间（effect scope 存活期间）跑；卸载即清理
 *   定时器与事件订阅。
 * - **事件刷新**：监听 `sessions.changed`，400ms 防抖后补一次读数，避免事件风暴
 *   把轮询节奏打乱。
 * - **诚实降级**：调用失败 / `ok !== true` → 成员清空 + 保留可读原因，交给视图
 *   显示「空态 + 说明」，绝不静默，也不保留上一轮的陈旧成员。
 */

import { computed, getCurrentScope, onScopeDispose, ref, watch } from 'vue'
import type { ComputedRef, Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

export const TEAM_MEMBERS_METHOD = 'team.members'
/** 轮询间隔（仅视图挂载期间生效）。 */
export const TEAM_BOARD_POLL_MS = 5_000
/** `sessions.changed` 事件合并窗口。 */
export const TEAM_BOARD_EVENT_DEBOUNCE_MS = 400

/**
 * 契约字段按线缆原样保留：kind / status 用 string 而不是封闭联合，
 * 这样后端将来扩展取值时视图会落到中性样式与原始文案，而不是谎报成已知状态。
 */
export interface TeamMember {
  id: string
  label: string
  role: string
  kind: string
  status: string
  currentStep: string | null
  toolCalls: number
  lastActivityMs: number | null
  tokensUsed: number | null
}

export interface TeamMembersPayload {
  ok?: boolean
  session?: string
  updatedAt?: number
  members?: unknown
  error?: string
  reason?: string
}

function textValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function finiteNumberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/**
 * 单条成员记录的归一化：缺 id 的行直接丢弃（无法作为卡片 key），
 * 数值字段只接受有限数字，其余如实降级为 0 / null。
 */
export function normalizeTeamMember(raw: unknown): TeamMember | null {
  if (!raw || typeof raw !== 'object') return null
  const row = raw as Record<string, unknown>
  const id = textValue(row.id)
  if (!id) return null
  const label = textValue(row.label) || textValue(row.role) || id
  const step = textValue(row.currentStep)
  const toolCalls = finiteNumberOrNull(row.toolCalls)
  return {
    id,
    label,
    role: textValue(row.role),
    kind: textValue(row.kind) || 'unknown',
    status: textValue(row.status) || 'unknown',
    currentStep: step || null,
    toolCalls: toolCalls === null ? 0 : Math.max(0, toolCalls),
    lastActivityMs: finiteNumberOrNull(row.lastActivityMs),
    tokensUsed: finiteNumberOrNull(row.tokensUsed),
  }
}

/** 整组归一化：非数组 → 空列表（空态，而不是报错造数据）。 */
export function normalizeTeamMembers(raw: unknown): TeamMember[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map(normalizeTeamMember)
    .filter((member): member is TeamMember => member !== null)
}

export interface TeamBoardState {
  members: Ref<TeamMember[]>
  supported: ComputedRef<boolean>
  loading: Ref<boolean>
  /** 非空表示最近一次读取失败/被后端回绝；视图据此显示空态 + 说明。 */
  error: Ref<string>
  updatedAt: Ref<number | null>
  refresh: () => Promise<void>
}

export function useTeamBoard(): TeamBoardState {
  const rpc = useRpcStore()

  const members = ref<TeamMember[]>([])
  const updatedAt = ref<number | null>(null)
  const loading = ref(false)
  const error = ref('')
  const supported = computed(() => rpc.supportsMethod(TEAM_MEMBERS_METHOD))

  let pollTimer: ReturnType<typeof setInterval> | null = null
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  let unsubscribe: (() => void) | null = null
  let inflight = false

  function clearPollTimer(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function clearDebounceTimer(): void {
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer)
      debounceTimer = null
    }
  }

  async function refresh(): Promise<void> {
    if (!supported.value || inflight) return
    inflight = true
    loading.value = true
    try {
      const payload = await rpc.call<TeamMembersPayload>(TEAM_MEMBERS_METHOD, {})
      if (!payload || payload.ok !== true) {
        members.value = []
        updatedAt.value = null
        error.value =
          textValue(payload?.error)
          || textValue(payload?.reason)
          || '网关未返回可用的 team.members 结果'
        return
      }
      members.value = normalizeTeamMembers(payload.members)
      updatedAt.value = finiteNumberOrNull(payload.updatedAt)
      error.value = ''
    } catch (err) {
      members.value = []
      updatedAt.value = null
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      inflight = false
      loading.value = false
    }
  }

  function scheduleEventRefresh(): void {
    if (!supported.value) return
    clearDebounceTimer()
    debounceTimer = setTimeout(() => {
      debounceTimer = null
      void refresh()
    }, TEAM_BOARD_EVENT_DEBOUNCE_MS)
  }

  function start(): void {
    if (pollTimer !== null) return
    // 事件订阅与轮询同生共死：只有门控打开时才存在。
    unsubscribe = rpc.on('sessions.changed', scheduleEventRefresh)
    void refresh()
    pollTimer = setInterval(() => void refresh(), TEAM_BOARD_POLL_MS)
  }

  function stop(): void {
    clearPollTimer()
    clearDebounceTimer()
    if (unsubscribe) {
      unsubscribe()
      unsubscribe = null
    }
  }

  function reset(): void {
    members.value = []
    updatedAt.value = null
    error.value = ''
  }

  watch(
    supported,
    (ok) => {
      if (ok) {
        start()
        return
      }
      // 门控关闭：停掉一切后台工作，并清空可能残留的状态。
      stop()
      reset()
    },
    { immediate: true },
  )

  if (getCurrentScope()) onScopeDispose(stop)

  return { members, supported, loading, error, updatedAt, refresh }
}
