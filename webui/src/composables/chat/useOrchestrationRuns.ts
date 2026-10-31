// 编排运行视图模型（P2·E 可见性）：把后端 `session.event.progress` 里的
// `kind: "orchestration"` 帧聚合成「一次编排 = 一条泳道」。
//
// 为什么需要它：蜂群 / 团队此前在后台跑，对话页只拿到一个 taskId —— 用户看不到
// 谁在做什么、调了哪些工具、哪一步失败，等于黑盒。后端现在按 phase 发布
// run_started / member_started / tool_call / tool_result / member_completed /
// lead_synthesizing / run_completed / run_failed，这里负责把一串离散事件还原成
// 可折叠的执行时间线。
//
// 契约（与 core/src/agent/subagent.rs::ProgressReporter 对齐）：
//   { kind: "orchestration", phase, runId, memberId?, role?, ... }
// 产线帧同形状但带 `engine: "line"`，在 `handleFrame` 入口被排除（见该处注释）。
// 事件归因用 meta.session（WS 帧的 meta 字段），而不是 payload.sessionKey ——
// 后者只在部分 RPC 响应里出现。
import { computed, getCurrentInstance, onBeforeUnmount, ref, watch } from 'vue'
import { notifyReadFailure } from '@/utils/readFailureNotice'

export type OrchestrationPhase =
  | 'run_started'
  | 'member_started'
  | 'member_thinking'
  | 'tool_call'
  | 'tool_result'
  | 'member_completed'
  | 'lead_synthesizing'
  | 'run_completed'
  | 'run_failed'

export interface OrchestrationThinkingRound {
  round: number
  text: string
}

export interface OrchestrationToolRow {
  id: string
  tool: string
  argsPreview: string
  outputPreview: string
  status: 'running' | 'ok' | 'error'
  elapsedMs: number | null
  at: string
}

export interface OrchestrationMember {
  id: string
  role: string
  roleLabel: string
  task: string
  status: 'running' | 'ok' | 'error'
  startedAtMs: number
  elapsedMs: number | null
  summary: string
  tools: OrchestrationToolRow[]
  /** 子代理各轮真实思考（后端 `member_thinking` 帧）。旧快照没有该字段。 */
  thinkingRounds?: OrchestrationThinkingRound[]
}

export interface OrchestrationRun {
  runId: string
  form: string
  goal: string
  status: 'running' | 'completed' | 'failed'
  startedAtMs: number
  endedAtMs: number | null
  memberEstimate: number
  members: OrchestrationMember[]
  synthesizing: boolean
  reply: string
  error: string
}

export type OrchestrationProgressFrame = {
  event?: string
  payload?: Record<string, unknown>
  meta?: { session?: string }
}

const MAX_RUNS = 6
const MAX_TOOLS_PER_MEMBER = 40
const MAX_THINKING_ROUNDS = 40

/* ── 刷新 / 重启后仍可见 ──────────────────────────────────────────────
 *
 * 真机缺陷：泳道卡片只活在内存里，刷新页面就消失，用户又回到「只看到一条最终
 * 答复」的黑盒状态。后端进度事件不落库（体积大、且属于过程态），所以在会话维度
 * 做 localStorage 快照：只存**已结束**的编排（running 的留着重启后会永远转圈），
 * 每个会话一个桶，切换会话自动切换视图。
 */
const STORAGE_VERSION = 1

function storageKey(session: string): string {
  return `marguerite.chat.orchRuns.v${STORAGE_VERSION}:${session}`
}

function isRunLike(value: unknown): value is OrchestrationRun {
  if (!value || typeof value !== 'object') return false
  const v = value as Record<string, unknown>
  return typeof v.runId === 'string'
    && (v.status === 'completed' || v.status === 'failed')
    && Array.isArray(v.members)
}

function readRuns(session: string): OrchestrationRun[] {
  if (!session) return []
  try {
    const raw = globalThis.localStorage?.getItem(storageKey(session))
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) {
      notifyReadFailure('orchestration-runs', 'shared.storage.orchestrationRunsReadFailed')
      return []
    }
    return parsed.filter(isRunLike).slice(0, MAX_RUNS)
  } catch {
    // 快照损坏 / 存储不可用：泳道会看起来「这次的编排从未发生过」。
    // 能恢复（清掉坏快照即可），所以用 toast 明确告知，而不是空白。
    notifyReadFailure('orchestration-runs', 'shared.storage.orchestrationRunsReadFailed')
    return []
  }
}

function writeRuns(session: string, items: OrchestrationRun[]): void {
  if (!session) return
  try {
    if (!items.length) {
      globalThis.localStorage?.removeItem(storageKey(session))
      return
    }
    globalThis.localStorage?.setItem(storageKey(session), JSON.stringify(items))
  } catch {
    // 配额满 / 隐私模式：静默降级为「刷新即失」，不影响对话。
  }
}

function str(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** 每条工具调用需要一个稳定 id 才能在列表里正确更新（后端不分配）。 */
let toolSeq = 0

/**
 * 每个会话的运行列表（**含 running**），进程内缓存。
 *
 * 真机缺陷：切走会话时 `runs.value` 被换成新会话的桶，正在跑的那张卡直接消失；
 * 切回来也回不来（running 从不落盘），这期间到达的终态帧又被会话过滤丢掉 ——
 * 这次编排在 UI 上等于从未发生过。改为按会话缓存：切会话只换视图，不丢列表。
 */
const sessionRuns = new Map<string, OrchestrationRun[]>()

/** 仅供测试：清空进程内会话缓存（localStorage 之外的跨用例脏状态）。 */
export function __resetOrchestrationRunsCache(): void {
  sessionRuns.clear()
}

export function useOrchestrationRuns(sessionKey: { readonly value: string }) {
  const runs = ref<OrchestrationRun[]>(
    sessionRuns.get(sessionKey.value) ?? readRuns(sessionKey.value),
  )
  const now = ref(Date.now())
  let ticker: ReturnType<typeof setInterval> | null = null
  let seq = 0

  // 切会话：只换视图。`flush: 'sync'` 是必须的 —— 切换后立刻到达的帧必须按新会话
  // 归约，异步 flush 会让它落到旧 key 的桶里。
  watch(
    () => sessionKey.value,
    (session, previous) => {
      stopTicker()
      if (previous) sessionRuns.set(previous, runs.value)
      runs.value = sessionRuns.get(session) ?? readRuns(session)
      now.value = Date.now()
      if (runs.value.some(run => run.status === 'running')) ensureTicker()
    },
    { flush: 'sync' },
  )

  // 只落已结束的编排：running 的持久化后重开会永远停在转圈。
  watch(
    runs,
    (items) => {
      const session = sessionKey.value
      // 进程内缓存保持最新：组件重挂载（离开对话页再回来）也要能拿回运行中的卡片。
      if (session) sessionRuns.set(session, items)
      writeRuns(session, items.filter(run => run.status !== 'running').slice(0, MAX_RUNS))
    },
    { deep: true },
  )

  function ensureTicker() {
    if (ticker) return
    ticker = setInterval(() => {
      now.value = Date.now()
      if (!runs.value.some(run => run.status === 'running')) stopTicker()
    }, 1000)
  }

  function stopTicker() {
    if (ticker) {
      clearInterval(ticker)
      ticker = null
    }
  }

  function findMember(run: OrchestrationRun, memberId: string): OrchestrationMember | undefined {
    return run.members.find(m => m.id === memberId)
  }

  /**
   * 把一帧编排进度归约进 `list`，返回被修改的列表；`null` 表示该帧被忽略。
   *
   * 抽成「对列表操作」而不是直接读写 `runs.value`，是因为非当前会话的帧也必须
   * 归约并缓存 —— 否则切走期间的进度会整段丢失（切回来只剩一张过期的卡）。
   */
  function applyProgress(
    list: OrchestrationRun[],
    payload: Record<string, unknown>,
  ): OrchestrationRun[] | null {
    const phase = str(payload.phase) as OrchestrationPhase
    const runId = str(payload.runId) || `run-local-${++seq}`
    const findRun = (id: string) => list.find(run => run.runId === id)

    if (phase === 'run_started') {
      const existing = findRun(runId)
      if (!existing) {
        const created: OrchestrationRun = {
          runId,
          form: str(payload.form) || 'swarm',
          goal: str(payload.goal),
          status: 'running',
          startedAtMs: Date.now(),
          endedAtMs: null,
          memberEstimate: num(payload.memberEstimate) ?? 0,
          members: [],
          synthesizing: false,
          reply: '',
          error: '',
        }
        // 新的在前：最近一次编排贴近日志关注点。
        return [created, ...list].slice(0, MAX_RUNS)
      }
      // 重连对账 / 服务端重放会再发一次 run_started：已存在的 run 不得被重置起点，
      // 更不得把已落定的终态改回 running（否则卡片会「完成 → 又运行」）。
      existing.form = str(payload.form) || existing.form
      existing.goal = str(payload.goal) || existing.goal
      existing.memberEstimate = num(payload.memberEstimate) ?? existing.memberEstimate
      return list
    }

    const run = findRun(runId)
    if (!run) return null
    const memberId = str(payload.memberId)
    const member = memberId ? findMember(run, memberId) : undefined
    const terminated = run.status === 'completed' || run.status === 'failed'

    if (phase === 'member_started') {
      // 终态之后的迟到帧（重放 / 乱序）不得再造出「运行中」的假成员行。
      if (terminated) return null
      if (memberId && !member) {
        run.members = [
          ...run.members,
          {
            id: memberId,
            role: str(payload.role),
            roleLabel: str(payload.roleLabel) || str(payload.role) || memberId.slice(0, 6),
            task: str(payload.task),
            status: 'running',
            startedAtMs: Date.now(),
            elapsedMs: null,
            summary: '',
            tools: [],
            thinkingRounds: [],
          },
        ]
      }
      return list
    }

    if (phase === 'member_thinking' && member) {
      // 终态后的迟到帧不追加（重放/乱序），否则已完成的成员会不断长出新思考。
      if (terminated) return null
      const text = str(payload.thinking).trim()
      if (!text) return null
      const round = num(payload.round) ?? (member.thinkingRounds?.length ?? 0) + 1
      const rounds = member.thinkingRounds ?? []
      // 同一轮重复帧（重连重放）覆盖而不是追加。
      member.thinkingRounds = [...rounds.filter(item => item.round !== round), { round, text }]
        .sort((a, b) => a.round - b.round)
        .slice(-MAX_THINKING_ROUNDS)
      return list
    }

    if (phase === 'tool_call' && member) {
      // 同上：已完成/失败之后不再追加运行中的工具行。
      if (terminated) return null
      const row: OrchestrationToolRow = {
        id: `tool-${++toolSeq}`,
        tool: str(payload.tool),
        argsPreview: str(payload.argsPreview),
        outputPreview: '',
        status: 'running',
        elapsedMs: null,
        at: new Date().toLocaleTimeString(),
      }
      member.tools = [...member.tools, row].slice(-MAX_TOOLS_PER_MEMBER)
      return list
    }

    if (phase === 'tool_result' && member) {
      const tool = str(payload.tool)
      // 结果帧不带宽带 id：回填该成员**最近一条同名且仍在运行**的工具行，
      // 多轮同名调用时才能正确配对（按名字找第一条会把结果贴错行）。
      const row = [...member.tools].reverse().find(t => t.tool === tool && t.status === 'running')
      if (row) {
        row.status = payload.ok === true ? 'ok' : 'error'
        row.outputPreview = str(payload.outputPreview)
        row.elapsedMs = num(payload.elapsedMs)
      }
      return list
    }

    if (phase === 'member_completed' && member) {
      member.status = payload.ok === true ? 'ok' : 'error'
      member.elapsedMs = num(payload.elapsedMs)
      member.summary = str(payload.summary)
      // 成员结束时仍标 running 的工具行统一收口，避免出现「永远转圈」的假运行态。
      member.tools = member.tools.map(t =>
        t.status === 'running' ? { ...t, status: member.status === 'ok' ? 'ok' : 'error' } : t,
      )
      return list
    }

    if (phase === 'lead_synthesizing') {
      run.synthesizing = true
      return list
    }

    if (phase === 'run_completed') {
      run.status = 'completed'
      run.synthesizing = false
      run.endedAtMs = Date.now()
      run.reply = str(payload.reply)
      const summaries = Array.isArray(payload.summaries) ? payload.summaries : []
      // 后端把成员摘要按顺序回传：没有单独 member_completed 的成员在这里补齐，
      // 保证「结论可见」而不是只有最终答复。
      // 终态即事实：丢失 member_completed 时成员行与工具行也必须收口 ——
      // 否则卡片头写着「已完成」，展开后成员/工具还在转圈。
      const ok = payload.ok !== false
      run.members = run.members.map((m, i) => {
        const summary = m.summary || (typeof summaries[i] === 'string' ? (summaries[i] as string) : '')
        const status: 'ok' | 'error' = ok ? 'ok' : 'error'
        return {
          ...m,
          summary,
          status: m.status === 'running' ? status : m.status,
          elapsedMs: m.elapsedMs ?? Math.max(0, Date.now() - m.startedAtMs),
          tools: m.tools.map(t =>
            t.status === 'running' ? { ...t, status } : t,
          ),
        }
      })
      return list
    }

    if (phase === 'run_failed') {
      run.status = 'failed'
      run.synthesizing = false
      run.endedAtMs = Date.now()
      run.error = str(payload.error)
      // 仍在运行的成员/工具一律判失败（已成功的保留原状态），避免整体失败时局部还在转圈。
      run.members = run.members.map(m => ({
        ...m,
        status: m.status === 'running' ? 'error' : m.status,
        elapsedMs: m.elapsedMs ?? Math.max(0, Date.now() - m.startedAtMs),
        tools: m.tools.map(t => (t.status === 'running' ? { ...t, status: 'error' as const } : t)),
      }))
      return list
    }

    return null
  }

  /**
   * 处理一帧编排进度。
   *
   * 非当前会话的帧**不再丢弃**：归约进该会话的缓存并落终态快照，
   * 用户切回去就能看到完整（且已收敛）的卡片。
   */
  function handleFrame(frame: OrchestrationProgressFrame) {
    const payload = frame.payload
    if (!payload || payload.kind !== 'orchestration') return
    // 产线（`line.run`）复用同一套相位词表发布进度，但它在对话流里有**自己的**
    // 回执面（LineRunPanel）。这里的泳道卡是「一次编排 = 一条泳道，标签是表单」，
    // 而 `form` 缺省会兜底成 `swarm` —— 拿它呈现一条 solo/panel/relay 产线就是说谎。
    // 后端为此在帧上钉了 `engine: "line"`（不新造事件名，见计划 §7.1），所以在这
    // 个入口按引擎归属分流，而不是在渲染层猜 runId 前缀。
    if (payload.engine === 'line') return
    const current = sessionKey.value
    const frameSession = str(frame.meta?.session) || current
    if (!frameSession) return
    const isCurrent = !current || frameSession === current
    const list = isCurrent
      ? runs.value
      : (sessionRuns.get(frameSession) ?? readRuns(frameSession))
    const next = applyProgress(list, payload)
    if (!next) return
    if (isCurrent) {
      runs.value = [...next]
      if (next.some(run => run.status === 'running')) ensureTicker()
      return
    }
    sessionRuns.set(frameSession, next)
    writeRuns(frameSession, next.filter(run => run.status !== 'running').slice(0, MAX_RUNS))
  }

  /**
   * 对账发现服务端**完全没有**这个 run 的任何记录（网关重启 / 后台任务被 kill）：
   * 如实收口为失败，而不是让卡片无限转圈。仅在覆盖窗口确实包含该 run 的起点时调用。
   */
  function markRunLost(runId: string, error: string) {
    const run = runs.value.find(r => r.runId === runId)
    if (!run || run.status !== 'running') return
    run.status = 'failed'
    run.synthesizing = false
    run.endedAtMs = Date.now()
    run.error = error
    run.members = run.members.map(m => ({
      ...m,
      status: m.status === 'running' ? 'error' : m.status,
      elapsedMs: m.elapsedMs ?? Math.max(0, Date.now() - m.startedAtMs),
      tools: m.tools.map(t => (t.status === 'running' ? { ...t, status: 'error' as const } : t)),
    }))
    runs.value = [...runs.value]
    sessionRuns.set(sessionKey.value, runs.value)
  }

  const activeCount = computed(() => runs.value.filter(r => r.status === 'running').length)

  function clear() {
    runs.value = []
    sessionRuns.delete(sessionKey.value)
    stopTicker()
  }

  // 只在组件 setup 内注册卸载钩子：单测与独立调用没有组件实例，注册会刷 Vue 告警。
  if (getCurrentInstance()) onBeforeUnmount(stopTicker)

  return { runs, now, activeCount, handleFrame, markRunLost, clear }
}
