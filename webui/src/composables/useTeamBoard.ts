/**
 * 团队看板状态层 —— `team.members` 契约的唯一消费点。
 *
 * 设计纪律：
 * - **门控**：先 `supportsMethod('team.members')`，网关没声明该方法就完全不发请求，
 *   视图渲染诚实空态（不编造成员）。
 * - **轮询**：5s 一次，只在视图挂载期间（effect scope 存活期间）跑；卸载即清理
 *   定时器与事件订阅。
 * - **事件刷新**：监听 `sessions.changed` 与 `session.event.progress`（编排帧），
 *   400ms 窗口内合并补一次读数。**已有待执行的合并窗口时不再重置它** ——
 *   否则蜂群持续吐帧会让读数被永久推后（饿死）。
 * - **诚实降级**：调用失败 / `ok !== true` → 成员清空 + 保留可读原因，交给视图
 *   显示「空态 + 说明」，绝不静默，也不保留上一轮的陈旧成员。
 *
 * ## 为什么还要订阅 `session.event.progress`
 *
 * `team.members` 是 `ProgressReporter::emit` 的**内存投影**（见
 * `core/src/orchestration/member_registry.rs`），而有三类真实存在的数据进不了那份投影：
 *   1. `member_thinking`（`core/src/agent/subagent.rs`）—— 投影没有这个 phase 分支，
 *      会被 `_ => {}` 丢弃；
 *   2. `member_completed` 的 `elapsedMs` / `summary` / `timedOut` / `cancelled` ——
 *      投影只读 `ok` 与 `tokensUsed`；
 *   3. 网关在超时 / panic 兜底时**直接 `bus.publish`** 的 `run_failed`
 *      （`core/src/gateway/team.rs`）—— 没走 `ProgressReporter::emit`，因此投影里
 *      lead 永远停在 `running`。
 * 这三类帧**都在同一条 WS 事件流上**，看板此前一个都没接。这里只做**前端消费**，
 * 不新增契约、不改 RPC 方法名、不加字段：只读事件里已经存在的载荷。
 * 拿不到的字段一律落「未报告」空态，绝不用 0 或猜测冒充（铁律 R-10）。
 */

import { computed, getCurrentScope, onScopeDispose, ref, watch } from 'vue'
import type { ComputedRef, Ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

export const TEAM_MEMBERS_METHOD = 'team.members'
/** 编排进度帧的事件名（与 `core/src/gateway/events.rs::envelope_to_frame` 对齐）。 */
export const TEAM_PROGRESS_EVENT = 'session.event.progress'
/** 轮询间隔（仅视图挂载期间生效）。 */
export const TEAM_BOARD_POLL_MS = 5_000
/** 事件合并窗口。 */
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
  /** 以下全部是 `member_registry::render` / `snapshot` **已经**在线缆上发的字段。 */
  runId?: unknown
  form?: unknown
  note?: unknown
  memberLimit?: unknown
  dropped?: unknown
  fromDisk?: unknown
}

/** 一次编排运行的看板级元信息（同样只来自既有响应字段）。 */
export interface TeamRunInfo {
  runId: string
  session: string
  /** 编排形态：team / swarm / solo / expert / subagent；未知取值原样透传。 */
  form: string
  /** `true` = 内存投影为空、回落磁盘快照（daemon 重启后的**上一次**运行）。 */
  fromDisk: boolean
  /** 投影缓冲容量（后端 `memberLimit`）。 */
  memberLimit: number | null
  /** 因缓冲满而未被投影的成员数（后端如实回传，不静默隐藏）。 */
  dropped: number
  /** 后端在没有运行记录时给出的可读原因（`snapshot` 的 `note` 字段）。 */
  note: string
}

export const EMPTY_RUN_INFO: TeamRunInfo = {
  runId: '',
  session: '',
  form: '',
  fromDisk: false,
  memberLimit: null,
  dropped: 0,
  note: '',
}

/** `member_registry::render` 给 lead 卡片用的 id 前缀（`lead:<runId>`）。 */
export function leadActivityKey(runId: string): string {
  return `lead:${runId}`
}

/** 卡片是否是被合成出来的 lead 行（`team.members` 的第一行恒为它）。 */
export function isLeadRow(member: TeamMember): boolean {
  return member.kind === 'lead' && member.id.startsWith('lead:')
}

function textValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function finiteNumberOrNull(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function boolOrNull(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
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
    // 0 是**真实计数**（后端从 member_started 起累加），不是「没上报」——
    // 只有 tokensUsed 这类后端明确回 null 的字段才走「未报告」空态。
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

/** 运行元信息归一化：缺字段一律落到空/中性值，不猜。 */
export function normalizeTeamRunInfo(payload: TeamMembersPayload | null | undefined): TeamRunInfo {
  if (!payload || typeof payload !== 'object') return { ...EMPTY_RUN_INFO }
  const dropped = finiteNumberOrNull(payload.dropped)
  return {
    runId: textValue(payload.runId),
    session: textValue(payload.session),
    form: textValue(payload.form),
    fromDisk: payload.fromDisk === true,
    memberLimit: finiteNumberOrNull(payload.memberLimit),
    dropped: dropped === null ? 0 : Math.max(0, Math.trunc(dropped)),
    note: textValue(payload.note),
  }
}

/* ── 编排进度帧的前端投影（成员动向）──────────────────────────────────── */

/**
 * 一个成员的**实时**动向。所有字段都只在**真的收到对应帧**时才有值：
 * `startedAtMs` 是前端收到 `member_started` 的墙钟（不是后端时间戳，故只在
 * 收到过该帧时存在），`elapsedMs` / `summary` 则直接取后端载荷。
 */
export interface MemberLiveActivity {
  key: string
  runId: string
  /** 最近一次**收到**该成员进度帧的时刻（`Date.now()`），用于陈旧帧判定。 */
  lastEventAtMs: number
  lastPhase: string
  /** 运行中的观测（member_started / tool_call / tool_result / member_thinking）。 */
  liveStatus: 'running' | null
  /** 终态观测（member_completed / run_completed / run_failed）。 */
  terminalStatus: 'finished' | 'failed' | null
  startedAtMs: number | null
  thinking: string
  thinkingRound: number | null
  lastTool: string
  lastToolOk: boolean | null
  lastToolElapsedMs: number | null
  elapsedMs: number | null
  summary: string
  timedOut: boolean | null
  cancelled: boolean | null
  error: string
  synthesizing: boolean
}

export function emptyMemberActivity(key: string, runId: string, atMs: number): MemberLiveActivity {
  return {
    key,
    runId,
    lastEventAtMs: atMs,
    lastPhase: '',
    liveStatus: null,
    terminalStatus: null,
    startedAtMs: null,
    thinking: '',
    thinkingRound: null,
    lastTool: '',
    lastToolOk: null,
    lastToolElapsedMs: null,
    elapsedMs: null,
    summary: '',
    timedOut: null,
    cancelled: null,
    error: '',
    synthesizing: false,
  }
}

export type MemberActivityMap = Readonly<Record<string, MemberLiveActivity>>

/**
 * 动向表当前属于哪一次编排：取**最近更新**的那一行的 runId。
 * 空表 → 空串（还没观察到任何帧）。
 */
function trackedRunId(map: MemberActivityMap): string {
  let latest: MemberLiveActivity | null = null
  for (const activity of Object.values(map)) {
    if (!activity.runId) continue
    if (!latest || activity.lastEventAtMs >= latest.lastEventAtMs) latest = activity
  }
  return latest?.runId ?? ''
}

/**
 * 把一帧编排进度归约进动向表；返回 `null` 表示这帧与看板无关（不改动画表）。
 *
 * 纯函数：不读全局、不读时钟 —— `receivedAtMs` 由调用方传入，便于单测。
 * 归约是**幂等**的（同 id 重放同一轮不会重复计数），因为重连补发会重放帧。
 */
export function reduceProgressFrame(
  current: MemberActivityMap,
  payload: unknown,
  receivedAtMs: number,
): MemberActivityMap | null {
  if (!payload || typeof payload !== 'object') return null
  const row = payload as Record<string, unknown>
  if (row.kind !== 'orchestration') return null
  const phase = textValue(row.phase)
  if (!phase) return null

  const runId = textValue(row.runId)
  const memberId = textValue(row.memberId)
  // 成员级帧按 memberId 归位；run 级帧（run_started / lead_synthesizing /
  // run_completed / run_failed，以及不带 memberId 的 tool_call）归到 lead 卡 ——
  // 这与 `member_registry::render` 给 lead 用的 id `lead:<runId>` 同构。
  const key = memberId || (runId ? leadActivityKey(runId) : '')
  if (!key) return null

  const previous = current[key] ?? emptyMemberActivity(key, runId, receivedAtMs)
  const next: MemberLiveActivity = {
    ...previous,
    runId: runId || previous.runId,
    lastEventAtMs: receivedAtMs,
    lastPhase: phase,
  }

  switch (phase) {
    case 'run_started': {
      // 新一轮开始：lead 回到运行中并清掉上一轮的终态痕迹。
      next.liveStatus = 'running'
      next.terminalStatus = null
      next.synthesizing = false
      const tracked = trackedRunId(current)
      if (runId && tracked && tracked !== runId) {
        // 换了一轮编排：上一轮其它成员的陈旧动向一并作废（与后端
        // member_registry 的 new-run 清空同语义 —— 留着会让用户以为它们还在动）。
        next.startedAtMs = receivedAtMs
        const pruned: Record<string, MemberLiveActivity> = {}
        for (const [existingKey, activity] of Object.entries(current)) {
          if (activity.runId === runId) pruned[existingKey] = activity
        }
        pruned[key] = next
        return pruned
      }
      if (runId && tracked !== runId) next.startedAtMs = receivedAtMs
      return { ...current, [key]: next }
    }
    case 'member_started': {
      next.liveStatus = 'running'
      next.terminalStatus = null
      // 只有真的收到过这一帧，卡片才有「开始时间」可显示。
      if (previous.startedAtMs === null || previous.terminalStatus !== null) {
        next.startedAtMs = receivedAtMs
      }
      return { ...current, [key]: next }
    }
    case 'member_thinking': {
      const text = textValue(row.thinking)
      if (!text) return null
      next.thinking = text
      next.thinkingRound = finiteNumberOrNull(row.round)
      if (!next.liveStatus && !next.terminalStatus) next.liveStatus = 'running'
      return { ...current, [key]: next }
    }
    case 'tool_call': {
      next.lastTool = textValue(row.tool) || 'tool'
      next.lastToolOk = null
      next.lastToolElapsedMs = null
      if (!next.terminalStatus) next.liveStatus = 'running'
      return { ...current, [key]: next }
    }
    case 'tool_result': {
      next.lastTool = textValue(row.tool) || next.lastTool || 'tool'
      next.lastToolOk = boolOrNull(row.ok)
      next.lastToolElapsedMs = finiteNumberOrNull(row.elapsedMs)
      if (!next.terminalStatus) next.liveStatus = 'running'
      return { ...current, [key]: next }
    }
    case 'member_completed': {
      // 与后端同语义：ok 缺省视为成功（`unwrap_or(true)`）。
      next.terminalStatus = row.ok === false ? 'failed' : 'finished'
      next.liveStatus = null
      next.elapsedMs = finiteNumberOrNull(row.elapsedMs)
      next.summary = textValue(row.summary)
      next.timedOut = boolOrNull(row.timedOut)
      next.cancelled = boolOrNull(row.cancelled)
      return { ...current, [key]: next }
    }
    case 'lead_synthesizing': {
      next.synthesizing = true
      next.liveStatus = 'running'
      return { ...current, [key]: next }
    }
    case 'run_completed': {
      next.terminalStatus = row.ok === false ? 'failed' : 'finished'
      next.liveStatus = null
      next.synthesizing = false
      return { ...current, [key]: next }
    }
    case 'run_failed': {
      // 网关的超时 / panic 兜底帧是**直接 publish 到总线**的，投影里没有它们，
      // 于是快照的 lead 会永远停在 running —— 这里就是它的收口点。
      next.terminalStatus = 'failed'
      next.liveStatus = null
      next.synthesizing = false
      const error = textValue(row.error)
      if (error) next.error = error
      return { ...current, [key]: next }
    }
    default:
      return null
  }
}

const TERMINAL_STATUSES = new Set(['finished', 'failed'])

/**
 * 快照状态与实时观测的合并规则（看板唯一的状态机）。
 *
 * - 实时帧只有在**比快照更新**时才有效 —— 快照本身可能已经反映了这一轮之后的
 *   状态（5s 轮询追上事件流），此时后到的重放/乱序帧不得把卡片改回旧状态。
 * - 终态观测（completed / failed）压过快照的 `running`；这正是网关兜底
 *   `run_failed` 唯一的收口来源。
 * - 快照已是终态时，非终态帧**不降级**它（重放不该让已完成的成员重新转圈）。
 * - `thinking` 是**纯前端状态**：后端 `status` 取值里没有它，只有真的收到
 *   `member_thinking` 帧（投影丢弃的那一类）才会出现，回答「这位此刻在思考」。
 */
export function resolveMemberStatus(
  snapshotStatus: string,
  activity: MemberLiveActivity | null | undefined,
  snapshotUpdatedAtMs: number | null,
): string {
  if (!activity) return snapshotStatus
  if (snapshotUpdatedAtMs !== null && activity.lastEventAtMs < snapshotUpdatedAtMs) {
    return snapshotStatus
  }
  if (activity.terminalStatus) return activity.terminalStatus
  if (TERMINAL_STATUSES.has(snapshotStatus)) return snapshotStatus
  if (activity.liveStatus) {
    return activity.lastPhase === 'member_thinking' ? 'thinking' : activity.liveStatus
  }
  return snapshotStatus
}

export interface TeamBoardState {
  members: Ref<TeamMember[]>
  supported: ComputedRef<boolean>
  loading: Ref<boolean>
  /** 非空表示最近一次读取失败/被后端回绝；视图据此显示空态 + 说明。 */
  error: Ref<string>
  updatedAt: Ref<number | null>
  /** 运行元信息（form / runId / dropped / fromDisk / note），全部来自既有响应字段。 */
  run: Ref<TeamRunInfo>
  /** 成员实时动向：按卡片 id（memberId 或 `lead:<runId>`）归位。 */
  activity: Ref<MemberActivityMap>
  refresh: () => Promise<void>
  /** 仅供单测注入一帧编排进度（真实链路由 `session.event.progress` 订阅驱动）。 */
  handleProgressPayload: (payload: unknown) => void
}

export function useTeamBoard(): TeamBoardState {
  const rpc = useRpcStore()

  const members = ref<TeamMember[]>([])
  const updatedAt = ref<number | null>(null)
  const loading = ref(false)
  const error = ref('')
  const run = ref<TeamRunInfo>({ ...EMPTY_RUN_INFO })
  const activity = ref<MemberActivityMap>({})
  const supported = computed(() => rpc.supportsMethod(TEAM_MEMBERS_METHOD))

  let pollTimer: ReturnType<typeof setInterval> | null = null
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  const unsubscribes: Array<() => void> = []
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
        run.value = { ...EMPTY_RUN_INFO }
        error.value =
          textValue(payload?.error)
          || textValue(payload?.reason)
          || '网关未返回可用的 team.members 结果'
        return
      }
      members.value = normalizeTeamMembers(payload.members)
      updatedAt.value = finiteNumberOrNull(payload.updatedAt)
      run.value = normalizeTeamRunInfo(payload)
      error.value = ''
    } catch (err) {
      members.value = []
      updatedAt.value = null
      run.value = { ...EMPTY_RUN_INFO }
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      inflight = false
      loading.value = false
    }
  }

  function scheduleEventRefresh(): void {
    if (!supported.value) return
    // 已经在窗口里了就不再重置：蜂群持续吐帧时，重置会让这次读数被永久推后。
    if (debounceTimer !== null) return
    debounceTimer = setTimeout(() => {
      debounceTimer = null
      void refresh()
    }, TEAM_BOARD_EVENT_DEBOUNCE_MS)
  }

  /**
   * 消费一帧编排进度。
   *
   * **不按会话过滤**：看板本来就是「最近一次运行」的视图（`team.members` 以空参调用，
   * 后端取最近更新的那个 run），键是 `memberId` / `lead:<runId>` —— 都是 run 内唯一的
   * id，因此不会串到别的卡片上。反过来，若在这里按 `meta.session` 与快照 session
   * 比对，「看着上一次的陈旧快照时用户新起了一个团队」就会把**所有**新帧挡掉，
   * 看板要等到下一次轮询才动 —— 那才是真的把动向弄丢了。
   */
  function handleProgressPayload(payload: unknown): void {
    const next = reduceProgressFrame(activity.value, payload, Date.now())
    if (!next) return
    activity.value = next
    // 帧意味着「有东西在动」：顺带补一次读数，让快照里的 toolCalls / currentStep
    // 不必等满 5s 轮询周期。
    scheduleEventRefresh()
  }

  function start(): void {
    if (pollTimer !== null) return
    // 事件订阅与轮询同生共死：只有门控打开时才存在。
    unsubscribes.push(rpc.on('sessions.changed', scheduleEventRefresh))
    unsubscribes.push(rpc.on(TEAM_PROGRESS_EVENT, handleProgressPayload))
    void refresh()
    pollTimer = setInterval(() => void refresh(), TEAM_BOARD_POLL_MS)
  }

  function stop(): void {
    clearPollTimer()
    clearDebounceTimer()
    while (unsubscribes.length) unsubscribes.pop()?.()
  }

  function reset(): void {
    members.value = []
    updatedAt.value = null
    error.value = ''
    run.value = { ...EMPTY_RUN_INFO }
    activity.value = {}
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

  return { members, supported, loading, error, updatedAt, run, activity, refresh, handleProgressPayload }
}
