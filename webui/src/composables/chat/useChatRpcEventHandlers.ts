import { computed, onScopeDispose, ref, watch, type Ref } from 'vue'
import type {
  ChatMessage,
  ChatPendingItem,
  ChatRunStatus,
  ChatRunStatusSource,
  ChatUsagePayload,
} from '@/types/chat'
import type {
  ArtifactPayload,
  CompactionPayload,
  CronResultPayload,
  EnsembleProgressPayload,
  InputDispositionPayload,
  MemoryRecallPayload,
  RouterDecisionPayload,
  SessionEventPayload,
  SessionMessagesSnapshotResponse,
  StepLoopPayload,
  StreamEventEnvelope,
  SubagentCompletionPayload,
  TextDeltaPayload,
  ToolDeltaPayload,
  ToolResultPayload,
  ToolUsePayload,
  WarningPayload,
} from '@/types/rpc'
import type { ChatRpcSubscriptionHandlers } from '@/composables/chat/useChatRpcSubscriptions'
import {
  isAuthoritativeSessionSubscription,
  type SessionSubscriptionOutcome,
} from '@/composables/chat/useChatSessionSubscription'
import type { SessionBootstrapRun } from '@/composables/chat/useChatSessionBootstrap'
import type { FrameInput } from '@/types/turnlog'
import type { StatusPart } from '@/types/parts'
import type { FoldLiveTurnMode } from '@/composables/chat/useChatTurnLog'
import {
  FINISHED_STREAM_TASK_ID,
  PENDING_STREAM_TASK_ID,
  STOPPED_STREAM_TASK_ID,
  acceptStreamSeq as decideStreamSeq,
  activeTaskGroupRunState as buildActiveTaskGroupRunState,
  frameSessionKey,
  isCurrentSessionPayload as payloadIsCurrentSession,
  isCurrentTaskPayload as payloadIsCurrentTask,
  isStaleEpoch as payloadIsStaleEpoch,
  payloadTaskId,
  sessionChangeIsStrongTerminal as payloadSessionChangeIsStrongTerminal,
  sessionChangeIsTerminal as payloadSessionChangeIsTerminal,
  sessionErrorMessage as eventSessionErrorMessage,
  taskGroupId as eventTaskGroupId,
  taskTerminalAsSessionEvent as normalizeTaskTerminalEvent,
  taskTerminalStatus as eventTaskTerminalStatus,
} from '@/utils/chat/streamEvents'
import { localizedChatErrorMessage } from '@/utils/chat/errors'
import { rehomePromotedSteerRows } from '@/utils/chat/historyMerge'

export interface ChatUsageAccumulator {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  cost: number | null
  routedTurns: number
  sessionSaved: number
}

export interface ChatRpcStreamApi {
  isStreaming: Ref<boolean>
  streamBubble: Ref<boolean>
  streamHasVisibleOutput: Ref<boolean>
  startStreaming: () => void
  endStreaming: (opts?: { reason?: string }) => void
  checkpointForUserMessage?: (turnId: string) => void
  appendDelta: (text: string, presentation?: 'intermediate' | 'answer') => void
  scheduleRender: () => void
  appendToolCall: (payload: ToolUsePayload) => void
  appendToolDelta: (payload: ToolDeltaPayload) => void
  appendToolResult: (payload: ToolResultPayload) => void
  appendArtifact: (payload: ArtifactPayload) => void
  reconcileFinalText: (finalText: string | null | undefined) => void
  resetLiveTurnState?: () => void
  resetStreamIdleTimer: () => void
  clearStreamIdleTimer: () => void
  setStreamActivity: (label: string) => void
  recordCompactionActivity?: (payload: CompactionPayload) => void
  showThinkingIndicator: () => void
  hideThinkingIndicator: () => void
  setThinkingContent: (text: string) => void
  // live-turn shadow log: the thinking ref lives here, so this composable appends
  // its own thinking frames into the stream-owned log after the legacy mutation.
  appendFrame: (frame: FrameInput) => void
  useReducer: Ref<FoldLiveTurnMode>
}

type ChatCompactionPlacement = 'activity' | 'standalone'
type ChatCompactionPresentationResult = boolean | ChatCompactionPlacement | void

export interface UseChatRpcEventHandlersOptions {
  sessionKey: Ref<string>
  currentEpoch: Ref<number>
  lastStreamSeq: Ref<number>
  activeTaskGroups: Ref<Set<string>>
  // Task id of the turn whose output the live stream is currently rendering.
  // Empty = unknown (no guard). Set when a fresh turn starts (see useChatSend)
  // and on task.running; reset on session switch.
  activeStreamTaskId: Ref<string>
  aborted: Ref<boolean>
  messages: Ref<ChatMessage[]>
  pendingQueue: Ref<ChatPendingItem[]>
  usageAccum: Ref<ChatUsageAccumulator>
  usageModel: Ref<string>
  stream: ChatRpcStreamApi
  normalizeRunStatus: (status: string) => string
  sessionRunStatus: (source: ChatRunStatusSource | null | undefined) => ChatRunStatus
  applySessionRunState: (source: ChatRunStatusSource | null | undefined) => void
  /**
   * True while an approval/clarify interrupt is on screen for this session.
   * Used to refuse the `sessions.changed` turn-end fallback (`occasion:
   * 'turn_end'` / `run_status: 'idle'`): an idle frame racing a pending approval
   * must never collapse the turn the user is still being asked about.
   */
  hasActiveInterrupt?: Ref<boolean>
  /** 重新挂上在屏的中断帧（审批/澄清）：实时快照重置 turn 后调用，防止卡片消失。 */
  reappendInterruptFrames?: () => void
  queueRouterDecision: (payload: RouterDecisionPayload) => void
  appendEnsembleProgress: (payload: EnsembleProgressPayload) => void
  markEnsembleHandoff: () => void
  flushPendingRouterDecision: () => void
  clearPendingRouterDecision: () => void
  handleRouterControlReplay: () => void
  showCompactionToast: (
    payload: CompactionPayload,
    meta?: Record<string, unknown>,
  ) => ChatCompactionPresentationResult
  getCompactionPlacement?: (compactionId: string) => ChatCompactionPlacement | undefined
  showWarningToast: (message: string) => void
  scheduleHistorySync: () => void
  schedulePendingDrainAfterTerminal: () => void
  popAllPendingIntoComposer: () => boolean
  restoreSteerIntoComposer?: (text: string) => void
  saveWidgetState: () => void
  subscribeSession?: () =>
    | boolean
    | void
    | SessionSubscriptionOutcome
    | Promise<boolean | void | SessionSubscriptionOutcome>
  onSessionSubscribed?: () => void | Promise<void>
  loadHistory?: () => void
  handleSessionConnectionState?: (state: string) => SessionBootstrapRun | undefined
  loadCurrentSessionUsage: () => void
  refreshRunModePreference?: () => void | Promise<void>
  /**
   * Called on `artifact.updated` (SPEC §5) so the owner can resync its artifact
   * index. Optional: the base handler stays inert when a call site does not care,
   * because unlike the streaming events this one carries no turn state — losing
   * it costs a stale sidebar, never a broken transcript.
   */
  resyncArtifacts?: (payload: SessionEventPayload) => void
}

type ChatDoneUsageFields = {
  input_tokens?: number
  output_tokens?: number
  cached_tokens?: number
  cache_write?: number
  cost_usd?: number
  model?: string
  text?: string
  text_snapshot?: string | null
  textSnapshot?: string | null
  model_usage_breakdown?: unknown
  modelUsageBreakdown?: unknown
  ensemble_trace?: unknown
  ensembleTrace?: unknown
  decision_id?: string
}

type ChatDoneUsagePayload = SessionEventPayload & ChatDoneUsageFields & {
  usage?: ChatDoneUsageFields
}

type BufferedTerminalEvent =
  | { kind: 'event'; event: string; payload: SessionEventPayload; priority: number }
  | { kind: 'session-change'; payload: SessionEventPayload; priority: number }

type BufferedTerminalEventInput =
  | { kind: 'event'; event: string; payload: SessionEventPayload; priority?: number }
  | { kind: 'session-change'; payload: SessionEventPayload; priority?: number }

type BufferedPendingStreamEvent = {
  event: string
  payload: SessionEventPayload
}

type BufferedPendingReplayEntry =
  | {
      kind: 'stream'
      event: string
      payload: SessionEventPayload
      order: number
    }
  | {
      kind: 'terminal'
      terminal: BufferedTerminalEvent
      payload: SessionEventPayload
      order: number
    }

const MAX_PENDING_TASK_BUCKETS = 8
const MAX_PENDING_STREAM_EVENTS_PER_TASK = 64
const SERVER_CLOCK_TOLERANCE_MS = 5_000
const MAX_TRUSTED_REASONING_AGE_MS = 60 * 60 * 1_000

const COMPACTION_TERMINAL_STATUSES = new Set([
  'completed',
  'skipped',
  'failed',
  'error',
  'cancelled',
  'timed_out',
  'stale',
  'emergency_ephemeral',
])

type LiveThinking = {
  text: string
  startedAt: number
  serverStartedAt: number | null
}

function trustedReasoningStartedAt(raw: unknown, now: number): number | null {
  if (typeof raw !== 'number' || !Number.isFinite(raw) || raw <= 0) return null
  if (raw > now + SERVER_CLOCK_TOLERANCE_MS) return null
  if (raw < now - MAX_TRUSTED_REASONING_AGE_MS) return null
  return raw
}

function trustedReasoningDoneAt(
  raw: unknown,
  serverStartedAt: number,
  now: number,
): number | null {
  if (typeof raw !== 'number' || !Number.isFinite(raw) || raw <= 0) return null
  if (raw < serverStartedAt) return null
  if (raw > now + SERVER_CLOCK_TOLERANCE_MS) return null
  if (raw - serverStartedAt > MAX_TRUSTED_REASONING_AGE_MS) return null
  return raw
}

function doneTextSnapshot(
  donePayload: ChatDoneUsagePayload,
  usagePayload: ChatDoneUsageFields,
): string | null {
  // A string snapshot is authoritative even when it is empty. Dataclass
  // serialization includes an unset optional field as null, so null must mean
  // "absent" and still permit the legacy nonempty `text` fallback.
  // Prefer any actual string across the outer and nested compatibility shapes.
  for (const source of [donePayload, usagePayload]) {
    for (const key of ['text_snapshot', 'textSnapshot'] as const) {
      if (Object.prototype.hasOwnProperty.call(source, key)) {
        const value = source[key]
        if (typeof value === 'string') return value
      }
    }
  }

  // Older gateways only sent `text`. Preserve their nonempty terminal
  // reconciliation behavior; legacy empty text always meant "fall back".
  for (const source of [usagePayload, donePayload]) {
    if (typeof source.text === 'string' && source.text) return source.text
  }
  return null
}

// A completed turn's measured thinking duration must survive the
// chat.history sync that replaces the messages array ~50ms after done.
// History rows carry the reasoning text but not the duration, so records
// are re-attached strictly by identity (reasoning or answer text) —
// never by timestamp proximity, which mis-bound reasoning to a
// neighbouring turn whenever a turn ran longer than the gap after it.
const REASONING_LOG_LIMIT = 20

interface TurnReasoningRecord {
  sessionKey: string
  text: string
  seconds: number
  messageText: string
}

export function useChatRpcEventHandlers(options: UseChatRpcEventHandlersOptions) {
  const {
    sessionKey,
    currentEpoch,
    lastStreamSeq,
    activeTaskGroups,
    activeStreamTaskId,
    aborted,
    messages,
    pendingQueue,
    usageAccum,
    usageModel,
    stream,
  } = options

  // Live thinking deltas for the current turn (session.event.thinking).
  const streamThinking = ref<LiveThinking | null>(null)
  const turnReasoningLog: TurnReasoningRecord[] = []
  const pendingTerminalEvents = new Map<string, BufferedTerminalEvent>()
  const pendingStreamEvents = new Map<string, BufferedPendingStreamEvent[]>()
  const settledTaskIds = new Set<string>()
  const restoredSteerRequestIds = new Set<string>()

  function compactionStatus(payload: CompactionPayload): string {
    const status = String(payload.status || '').toLowerCase()
    if (status) return status
    if (Object.prototype.hasOwnProperty.call(payload, 'compacted')) {
      return payload.compacted ? 'completed' : 'skipped'
    }
    return ''
  }

  function payloadCompactionId(payload: CompactionPayload): string {
    return String(payload.compaction_id || payload.compactionId || '').trim()
  }

  function compactionTerminalActivityState(status: string): StatusPart['state'] | undefined {
    if (status === 'completed' || status === 'emergency_ephemeral') return 'completed'
    if (status === 'skipped') return 'skipped'
    if (status === 'stale' || status === 'cancelled') return 'cancelled'
    if (status === 'failed' || status === 'error' || status === 'timed_out') return 'failed'
    return undefined
  }

  function settleCommittedCompactionActivity(payload: CompactionPayload): boolean {
    const id = payloadCompactionId(payload)
    const state = compactionTerminalActivityState(compactionStatus(payload))
    if (!id || !state) return false

    for (let messageIndex = messages.value.length - 1; messageIndex >= 0; messageIndex -= 1) {
      const message = messages.value[messageIndex]
      if (message?.role !== 'assistant' || !message.statusHistory) continue
      for (let statusIndex = message.statusHistory.length - 1; statusIndex >= 0; statusIndex -= 1) {
        const marker = message.statusHistory[statusIndex]
        if (marker?.action !== 'context_compaction' || marker.id !== id) continue
        message.statusHistory[statusIndex] = {
          ...marker,
          state,
          // The lifecycle stays anchored where its started frame first appeared.
          at: marker.at,
        }
        return true
      }
    }
    return false
  }

  function trackedLateCompactionPlacement(
    payload: CompactionPayload,
  ): ChatCompactionPlacement | undefined {
    if (activeStreamTaskId.value !== FINISHED_STREAM_TASK_ID) return undefined
    if (!isCurrentSessionPayload(payload)) return undefined
    if (!COMPACTION_TERMINAL_STATUSES.has(compactionStatus(payload))) return undefined
    const id = payloadCompactionId(payload)
    if (!id) return undefined
    return options.getCompactionPlacement?.(id)
  }

  function bufferedStreamSeq(payload: SessionEventPayload): number | null {
    const sequence = payload.stream_seq
    return typeof sequence === 'number' && Number.isFinite(sequence) ? sequence : null
  }

  function withoutBufferedStreamSeq(payload: SessionEventPayload): SessionEventPayload {
    const replayPayload = { ...payload }
    delete replayPayload.stream_seq
    return replayPayload
  }

  function comparePendingReplayEntries(
    left: BufferedPendingReplayEntry,
    right: BufferedPendingReplayEntry,
  ): number {
    const leftSequence = bufferedStreamSeq(left.payload)
    const rightSequence = bufferedStreamSeq(right.payload)
    if (leftSequence !== null && rightSequence !== null && leftSequence !== rightSequence) {
      return leftSequence - rightSequence
    }
    // Mixed-version gateways may omit sequence numbers. Preserve their old
    // stream-before-terminal behavior while still sorting every numbered frame.
    if (leftSequence !== null && rightSequence === null) {
      return right.kind === 'terminal' ? -1 : 1
    }
    if (leftSequence === null && rightSequence !== null) {
      return left.kind === 'terminal' ? 1 : -1
    }
    if (leftSequence !== null && leftSequence === rightSequence && left.kind !== right.kind) {
      // A terminal owns the cursor for a shared sequence, after all visible
      // frames at that sequence have been applied.
      return left.kind === 'terminal' ? 1 : -1
    }
    return left.order - right.order
  }

  function terminalEventPriority(event: string): number {
    if (event === 'session.event.done' || event === 'chat.done' || event.endsWith('.error')) return 3
    if (eventTaskTerminalStatus(event)) return 2
    return 0
  }

  function isTerminalEvent(event: string): boolean {
    if (eventTaskTerminalStatus(event)) return true
    if (event === 'session.event.done' || event === 'chat.done') return true
    return event.endsWith('.error') && !event.includes('.task_group.')
  }

  function bufferPendingTerminalEvent(
    entry: BufferedTerminalEventInput,
  ): boolean {
    if (activeStreamTaskId.value !== PENDING_STREAM_TASK_ID) return false
    if (!isCurrentSessionPayload(entry.payload)) return false
    if (isStaleEpoch(entry.payload)) return false
    const taskId = payloadTaskId(entry.payload)
    if (!taskId) return false
    const priority = entry.priority ?? (entry.kind === 'event' ? terminalEventPriority(entry.event) : 1)
    const existing = pendingTerminalEvents.get(taskId)
    if (!existing || priority >= existing.priority) {
      if (!existing && pendingTerminalEvents.size >= MAX_PENDING_TASK_BUCKETS) {
        const oldestTaskId = pendingTerminalEvents.keys().next().value
        if (typeof oldestTaskId === 'string') pendingTerminalEvents.delete(oldestTaskId)
      }
      pendingTerminalEvents.set(taskId, { ...entry, priority } as BufferedTerminalEvent)
    }
    return true
  }

  function markTaskSettled(payload: SessionEventPayload) {
    const taskId = payloadTaskId(payload)
    if (taskId) settledTaskIds.add(taskId)
    pendingTerminalEvents.clear()
    pendingStreamEvents.clear()
  }

  function bufferPendingStreamEvent(
    event: string,
    payload: SessionEventPayload,
  ): boolean {
    if (activeStreamTaskId.value !== PENDING_STREAM_TASK_ID) return false
    if (!isCurrentSessionPayload(payload)) return false
    if (isStaleEpoch(payload)) return false
    const taskId = payloadTaskId(payload)
    if (!taskId) return false

    let buffered = pendingStreamEvents.get(taskId)
    if (!buffered) {
      if (pendingStreamEvents.size >= MAX_PENDING_TASK_BUCKETS) {
        const oldestTaskId = pendingStreamEvents.keys().next().value
        if (typeof oldestTaskId === 'string') pendingStreamEvents.delete(oldestTaskId)
      }
      buffered = []
      pendingStreamEvents.set(taskId, buffered)
    }
    if (buffered.length >= MAX_PENDING_STREAM_EVENTS_PER_TASK) buffered.shift()
    buffered.push({ event, payload })
    return true
  }

  function replayPendingStreamEvent(entry: BufferedPendingStreamEvent) {
    const { event, payload } = entry
    if (event === 'session.event.text_delta') {
      handleRpcTextDelta(payload as TextDeltaPayload)
    } else if (event === 'session.event.tool_use_start') {
      handleRpcToolUseStart(payload as ToolUsePayload)
    } else if (event === 'session.event.tool_use_delta') {
      handleRpcToolUseDelta(payload as ToolDeltaPayload)
    } else if (event === 'session.event.tool_result') {
      handleRpcToolResult(payload as ToolResultPayload)
    } else if (event === 'session.event.artifact') {
      handleRpcArtifact(payload as ArtifactPayload)
    } else if (event === 'session.event.state_change') {
      handleRpcStateChange(payload)
    } else if (event === 'session.event.run_heartbeat') {
      handleRpcRunHeartbeat(payload)
    } else if (event === 'session.event.router_decision') {
      handleRpcRouterDecision(payload as RouterDecisionPayload)
    } else if (event === 'session.event.ensemble_progress') {
      handleRpcEnsembleProgress(payload as EnsembleProgressPayload)
    } else if (event === 'session.event.router_control_replay') {
      handleRpcRouterControlReplay(payload)
    } else if (event === 'session.event.compaction') {
      // A live snapshot is the authoritative base for the active stream, not
      // historical replay. Compaction deliberately ignores replayed
      // non-terminal events, so mark this as live to restore the busy/Stop
      // state before subscribing from snapshot.current_stream_seq.
      handleRpcCompaction(payload as CompactionPayload, {
        authoritativeLive: true,
        replayed: false,
      })
    } else if (event === 'session.event.thinking') {
      handleRpcAny(event, payload)
    }
  }

  function restoreLiveTurnSnapshot(snapshot: SessionMessagesSnapshotResponse) {
    if (!snapshot || snapshot.key !== sessionKey.value) return

    stream.resetLiveTurnState?.()
    clearLiveThinking()
    pendingTerminalEvents.clear()
    pendingStreamEvents.clear()
    settledTaskIds.clear()
    options.clearPendingRouterDecision()
    activeStreamTaskId.value = typeof snapshot.task_id === 'string'
      ? snapshot.task_id
      : ''

    for (const entry of snapshot.events || []) {
      if (!entry || typeof entry.event !== 'string') continue
      const payload = { ...(entry.payload || {}) }
      // Snapshot events retain their original sequence for diagnostics, but
      // they form an authoritative base rather than fresh deltas. Replaying
      // them through the normal render handlers without the old sequence
      // rebuilds the bubble even when this client already had a newer cursor.
      delete payload.stream_seq
      replayPendingStreamEvent({ event: entry.event, payload })
    }

    if (
      typeof snapshot.current_stream_seq === 'number'
      && Number.isFinite(snapshot.current_stream_seq)
    ) {
      lastStreamSeq.value = Math.max(0, snapshot.current_stream_seq)
    }
    // 快照重置了 live turn，但审批/澄清不是流事件 —— 不重新挂上就会「卡片凭空消失」
    // （badge 仍显示 pending，用户却无法裁决）。fold 按 approvalId 去重，重复挂安全。
    options.reappendInterruptFrames?.()
  }

  function replayPendingTerminalEvent(entry: BufferedTerminalEvent) {
    if (entry.kind === 'session-change') {
      handleRpcSessionsChanged(entry.payload)
    } else {
      handleRpcAny(entry.event, entry.payload)
    }
  }

  function bindActiveStreamTask(taskId: string) {
    if (!taskId) return
    const bufferedTerminal = pendingTerminalEvents.get(taskId)
    const bufferedStream = pendingStreamEvents.get(taskId) || []
    // One chat.send response resolves one PENDING window. Drop every other
    // task's early frames so a late prior task can never be consumed by a
    // future send that happens to reuse this session.
    pendingTerminalEvents.clear()
    pendingStreamEvents.clear()
    if (settledTaskIds.has(taskId)) {
      activeStreamTaskId.value = FINISHED_STREAM_TASK_ID
      return
    }
    activeStreamTaskId.value = taskId

    const replayEntries: BufferedPendingReplayEntry[] = bufferedStream.map(
      (entry, order) => ({ kind: 'stream', ...entry, order }),
    )
    if (bufferedTerminal) {
      replayEntries.push({
        kind: 'terminal',
        terminal: bufferedTerminal,
        payload: bufferedTerminal.payload,
        order: bufferedStream.length,
      })
    }
    replayEntries.sort(comparePendingReplayEntries)

    const terminalSequence = bufferedTerminal
      ? bufferedStreamSeq(bufferedTerminal.payload)
      : null
    let taskTerminalReplayed = false
    for (const entry of replayEntries) {
      if (entry.kind === 'terminal') {
        replayPendingTerminalEvent(entry.terminal)
        taskTerminalReplayed = true
        continue
      }

      const sequence = bufferedStreamSeq(entry.payload)
      const sharesTerminalSequence = terminalSequence !== null && sequence === terminalSequence
      const maintenanceAfterTerminal = taskTerminalReplayed
        && entry.event === 'session.event.compaction'
      // Let the terminal own a shared cursor. A tracked compaction terminal may
      // still close its existing UI after task completion, but it must not move
      // the task cursor past the terminal that closed the stream.
      const payload = sharesTerminalSequence || maintenanceAfterTerminal
        ? withoutBufferedStreamSeq(entry.payload)
        : entry.payload
      replayPendingStreamEvent({ event: entry.event, payload })
    }
  }

  // 1s ticker so the live "Thinking · Ns" label advances on a clock while
  // reasoning is open, not only when a new reasoning delta happens to arrive.
  const elapsedTick = ref(0)
  let elapsedTimer: ReturnType<typeof setInterval> | null = null
  watch(
    () => stream.isStreaming.value && !!streamThinking.value,
    (active) => {
      if (active && !elapsedTimer) {
        elapsedTimer = setInterval(() => { elapsedTick.value++ }, 1000)
      } else if (!active && elapsedTimer) {
        clearInterval(elapsedTimer)
        elapsedTimer = null
      }
    },
  )
  onScopeDispose(() => { if (elapsedTimer) { clearInterval(elapsedTimer); elapsedTimer = null } })

  const streamThinkingText = computed(() => streamThinking.value?.text || '')
  // Recomputed per delta AND on the 1s tick so the label keeps pace between
  // deltas; the final "Thought for Ns" uses the measured wall clock at done.
  const streamThinkingElapsedText = computed(() => {
    elapsedTick.value
    const current = streamThinking.value
    if (!current) return ''
    const seconds = Math.max(0, Math.floor((Date.now() - current.startedAt) / 1000))
    return `${seconds}s`
  })

  function appendThinkingDelta(text: string, payload: SessionEventPayload) {
    // 空/纯空白帧不进入思考流：否则会渲染出一个没有正文的「思考」折叠块，
    // 用户点开只能看到标题（真机截图里的空思考卡）。
    if (!text || !text.trim()) return
    if (!stream.isStreaming.value) stream.startStreaming()
    // 编排叙述（`kind: "narration"`）已在上游被丢弃，这里拿到的都是模型真实推理增量，
    // 按原样逐段拼接（绝不插分隔符，那会把推理链切断）。
    const current = streamThinking.value
    if (current) {
      streamThinking.value = { ...current, text: current.text + text }
    } else {
      const now = Date.now()
      const serverStartedAt = trustedReasoningStartedAt(payload.started_at, now)
      streamThinking.value = {
        text,
        startedAt: serverStartedAt ?? now,
        serverStartedAt,
      }
    }
    // Mirror the genuine reasoning fragment into the stream's thinking
    // indicator so it renders real content instead of a placeholder.
    stream.setThinkingContent(streamThinking.value.text)
    // The fold concats the same text into its thinkingText. Gating already
    // passed upstream (handleRpcAny), so this frame mirrors an accepted delta.
    if (stream.useReducer.value) stream.appendFrame({ kind: 'thinking', text, at: Date.now() })
    // Reasoning growth must re-pin the thread to the bottom just like answer
    // text and tool deltas. Schedule the same batched render/scroll flush so a
    // long thinking phase keeps following the live turn instead of only
    // snapping down once answer text starts streaming.
    stream.scheduleRender()
  }

  function clearLiveThinking() {
    streamThinking.value = null
    stream.setThinkingContent('')
  }

  // Walk recorded turn reasonings (newest first) and re-bind each to the
  // newest unclaimed assistant message it identifies: a row that already
  // carries this record's reasoning text gets its measured duration
  // restored, and a row without reasoning is claimed via its answer text
  // (covers history rows that predate the reasoning backfill). A record
  // with no identity match attaches nowhere — losing a duration beats
  // showing one turn's reasoning under another turn's answer. Idempotent:
  // safe to run after every history replacement.
  function attachTurnReasoning() {
    if (turnReasoningLog.length === 0) return
    const list = messages.value
    const claimed = new Set<ChatMessage>()
    for (let r = turnReasoningLog.length - 1; r >= 0; r--) {
      const record = turnReasoningLog[r]
      if (record.sessionKey !== sessionKey.value) continue
      for (let i = list.length - 1; i >= 0; i--) {
        const msg = list[i]
        if (msg.role !== 'assistant' || claimed.has(msg)) continue
        const carriesRecordReasoning = msg.reasoning?.text === record.text
        const matchesAnswerText =
          !msg.reasoning && record.messageText !== '' && msg.text.trim() === record.messageText
        if (!carriesRecordReasoning && !matchesAnswerText) continue
        claimed.add(msg)
        msg.reasoning = { text: record.text, seconds: record.seconds }
        break
      }
    }
  }

  function recordTurnReasoning(text: string, seconds: number, messageText: string) {
    turnReasoningLog.push({ sessionKey: sessionKey.value, text, seconds, messageText })
    if (turnReasoningLog.length > REASONING_LOG_LIMIT) {
      turnReasoningLog.splice(0, turnReasoningLog.length - REASONING_LOG_LIMIT)
    }
    attachTurnReasoning()
  }

  function doneUsagePayload(donePayload: ChatDoneUsagePayload): ChatUsagePayload | undefined {
    const raw = (donePayload.usage || donePayload || {}) as Record<string, unknown>
    if (!raw || typeof raw !== 'object') return undefined
    const usage = { ...raw } as ChatUsagePayload
    const direct = donePayload as Record<string, unknown>
    if (direct.model_usage_breakdown != null && usage.model_usage_breakdown == null) {
      usage.model_usage_breakdown = direct.model_usage_breakdown as never
    }
    if (direct.modelUsageBreakdown != null && usage.modelUsageBreakdown == null) {
      usage.modelUsageBreakdown = direct.modelUsageBreakdown as never
    }
    if (direct.ensemble_trace != null && usage.ensemble_trace == null) {
      usage.ensemble_trace = direct.ensemble_trace as never
    }
    if (direct.ensembleTrace != null && usage.ensembleTrace == null) {
      usage.ensembleTrace = direct.ensembleTrace as never
    }
    return usage
  }

  watch(sessionKey, () => {
    streamThinking.value = null
    turnReasoningLog.length = 0
    pendingTerminalEvents.clear()
    pendingStreamEvents.clear()
    settledTaskIds.clear()
    // The newly shown session has its own (possibly running) task; forget the
    // previous session's active task so we stay lenient until it re-asserts.
    activeStreamTaskId.value = ''
  })
  watch(activeStreamTaskId, (taskId) => {
    if (taskId === PENDING_STREAM_TASK_ID) {
      pendingTerminalEvents.clear()
      pendingStreamEvents.clear()
      settledTaskIds.clear()
    } else if (!taskId || taskId === FINISHED_STREAM_TASK_ID || taskId === STOPPED_STREAM_TASK_ID) {
      pendingTerminalEvents.clear()
      pendingStreamEvents.clear()
    }
  }, { flush: 'sync' })

  function isStaleEpoch(payload: StreamEventEnvelope): boolean {
    return payloadIsStaleEpoch(payload, currentEpoch.value)
  }

  function isCurrentSessionPayload(payload: StreamEventEnvelope): boolean {
    return payloadIsCurrentSession(payload, sessionKey.value)
  }

  // Drop late events tagged with a different task than the one rendering now,
  // so a stale turn's tool_use/error/done can't leak into the current turn
  // (issue #344). Lenient: untagged events and unknown active task pass.
  function isCurrentTaskPayload(payload: StreamEventEnvelope): boolean {
    return payloadIsCurrentTask(payload, activeStreamTaskId.value)
  }

  function acceptStreamSeq(payload: StreamEventEnvelope): boolean {
    const decision = decideStreamSeq(payload, sessionKey.value, lastStreamSeq.value)
    if (decision.accepted) lastStreamSeq.value = decision.nextStreamSeq
    return decision.accepted
  }

  /**
   * 本轮的流是否已经结算（done / error / 中止）。
   *
   * 结算后 `activeStreamTaskId` 落在 `FINISHED`/`STOPPED` 哨兵上，此时**不允许**任何
   * 迟到事件把流重新拉起 —— 见 [`restartStreamingForLiveEvent`]。
   */
  function turnIsSettled(): boolean {
    return activeStreamTaskId.value === FINISHED_STREAM_TASK_ID
      || activeStreamTaskId.value === STOPPED_STREAM_TASK_ID
  }

  /**
   * 仅在「本轮仍在进行」时按事件恢复流渲染。
   *
   * 背景（真机现象）：一轮以 `session.event.error`（如 401）收尾后，后端那一轮已经死了、
   * 不会再有事件；此时若再来一条 `state_change` / `ensemble_progress` 生命周期帧，旧代码
   * 会 `startStreaming()` 把流重新拉起并重新武装空闲看门狗（`DEFAULT_STREAM_IDLE_TIMEOUT_MS`
   * = 630s）。没有任何生产者再发事件 → 用户先看到失败卡，再干等满 630 秒才被告知
   * 「Response timed out -- no events received for 630s」——失败信息被一条假的超时盖住。
   *
   * 迟到的生命周期帧仍会刷新 seq/运行态（不丢信息），只是不再复活渲染。
   */
  function restartStreamingForLiveEvent() {
    if (stream.isStreaming.value || turnIsSettled()) return
    stream.startStreaming()
  }

  function activeTaskGroupRunState(payload: SessionEventPayload = {}) {
    return buildActiveTaskGroupRunState(payload, activeTaskGroups.value.size)
  }

  function noteTaskGroupActive(payload: SessionEventPayload) {
    const gid = eventTaskGroupId(payload)
    if (gid) activeTaskGroups.value.add(gid)
    options.applySessionRunState(activeTaskGroupRunState(payload))
  }

  function noteTaskGroupTerminal(payload: SessionEventPayload, terminalStatus: string) {
    const gid = eventTaskGroupId(payload)
    if (gid) activeTaskGroups.value.delete(gid)
    if (activeTaskGroups.value.size > 0) {
      options.applySessionRunState(activeTaskGroupRunState(payload))
      return
    }
    options.applySessionRunState({
      run_status: terminalStatus === 'failed' ? 'failed' : 'idle',
      last_task: { ...(payload || {}), status: terminalStatus },
    })
    if (!stream.isStreaming.value) {
      options.schedulePendingDrainAfterTerminal()
    }
  }

  function sessionChangeIsTerminal(payload: SessionEventPayload): boolean {
    return payloadSessionChangeIsTerminal(payload, options.normalizeRunStatus)
  }

  function sessionChangeIsStrongTerminal(payload: SessionEventPayload): boolean {
    return payloadSessionChangeIsStrongTerminal(payload, options.normalizeRunStatus)
  }

  function isStoppedCancelledTerminalEvent(terminalStatus: string, payload: SessionEventPayload): boolean {
    return (
      activeStreamTaskId.value === STOPPED_STREAM_TASK_ID &&
      isCurrentSessionPayload(payload) &&
      terminalStatus === 'cancelled'
    )
  }

  function isStoppedTerminalSessionChange(payload: SessionEventPayload): boolean {
    if (activeStreamTaskId.value !== STOPPED_STREAM_TASK_ID) return false
    if (!isCurrentSessionPayload(payload)) return false
    if (!sessionChangeIsTerminal(payload)) return false
    const state = options.sessionRunStatus(payload)
    return state.status === 'cancelled' || state.status === 'interrupted'
  }

  function syncTerminalSessionChange(payload: SessionEventPayload = {}) {
    if (!isCurrentSessionPayload(payload)) return false
    activeTaskGroups.value.clear()
    const state = options.sessionRunStatus(payload)
    const interrupted = state.status === 'cancelled' || state.status === 'interrupted'
    if (stream.isStreaming.value) stream.endStreaming(interrupted ? { reason: 'aborted' } : undefined)
    options.applySessionRunState(payload)
    options.scheduleHistorySync()
    if (interrupted) {
      options.popAllPendingIntoComposer()
    } else {
      options.schedulePendingDrainAfterTerminal()
    }
    activeStreamTaskId.value = FINISHED_STREAM_TASK_ID
    return true
  }

  function handleRpcTextDelta(payload: TextDeltaPayload) {
    if (isStaleEpoch(payload)) return
    if (bufferPendingStreamEvent('session.event.text_delta', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    options.markEnsembleHandoff()
    const presentation = payload.presentation
    if (presentation === 'intermediate' || presentation === 'answer') {
      stream.appendDelta(payload.text || '', presentation)
    } else {
      // Compatibility with older gateways that predate semantic text roles.
      stream.appendDelta(payload.text || '')
    }
  }

  function handleRpcToolUseStart(payload: ToolUsePayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.tool_use_start', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    options.markEnsembleHandoff()
    // payload.approval 是后端引擎（PermissionMatrix）的**真实裁决**
    // （approved / pending / denied）；原样交给 stream，缺失即不渲染徽章。
    stream.appendToolCall(payload)
  }

  function handleRpcToolUseDelta(payload: ToolDeltaPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.tool_use_delta', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    options.markEnsembleHandoff()
    stream.appendToolDelta(payload)
  }

  function handleRpcToolResult(payload: ToolResultPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.tool_result', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    stream.appendToolResult(payload)
  }

  function handleRpcArtifact(payload: ArtifactPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.artifact', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    stream.appendArtifact(payload)
  }

  /**
   * `artifact.updated` (SPEC §5) — a file was registered, rewritten by an inline
   * edit, or replaced by a regenerating turn.
   *
   * Deliberately filtered ONLY by session, unlike `session.event.artifact`. That
   * sibling gates on `isCurrentTaskPayload` + `acceptStreamSeq` because it is a
   * turn-scoped stream event, and those same gates are why artifacts went
   * missing from the sidebar: an artifact registered after the turn's task was
   * cleared, or written by an inline edit that belongs to no task at all, has no
   * current task to match and was silently dropped. This event exists precisely
   * to cover those cases, so gating it the same way would reintroduce the bug.
   *
   * It also must survive `aborted` — cancelling a turn does not un-write files
   * that already landed on disk.
   */
  function handleRpcArtifactUpdated(payload: SessionEventPayload) {
    if (!payload) return
    if (!payloadIsCurrentSession(payload, sessionKey.value)) return
    options.resyncArtifacts?.(payload)
  }

  function handleRpcStateChange(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!payload || aborted.value) return
    if (bufferPendingStreamEvent('session.event.state_change', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    stream.resetStreamIdleTimer()
    const to = payload.to_state || payload.toState || ''
    const activeState = ['thinking', 'streaming', 'tool_calling', 'tool_use', 'running'].includes(String(to))
    if (activeState) restartStreamingForLiveEvent()
    if (!stream.isStreaming.value) return
    if (activeState) options.markEnsembleHandoff()
    if (to === 'thinking') {
      if (stream.streamBubble.value && !stream.streamHasVisibleOutput.value) {
        stream.setStreamActivity('Planning next step')
      } else if (!stream.streamBubble.value) {
        stream.showThinkingIndicator()
      }
    } else if (to === 'streaming' && stream.streamBubble.value && !stream.streamHasVisibleOutput.value) {
      stream.setStreamActivity('Model is generating')
    } else if ((to === 'tool_calling' || to === 'tool_use') && stream.streamBubble.value && !stream.streamHasVisibleOutput.value) {
      stream.setStreamActivity('Preparing tool call')
    } else if (to && stream.streamBubble.value && !stream.streamHasVisibleOutput.value) {
      stream.setStreamActivity('Still running')
    }
  }

  function handleRpcRunHeartbeat(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.run_heartbeat', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    restartStreamingForLiveEvent()
    stream.resetStreamIdleTimer()
    if (stream.streamBubble.value && !stream.streamHasVisibleOutput.value) {
      const phase = String(payload.phase || '')
      // Generic keepalives prove liveness but are NOT phase transitions, so they
      // must not overwrite the activity label ('channel' from the Python
      // channel wrapper; 'turn' from the Rust gateway's turn heartbeat).
      const isPhaseAgnosticKeepalive = phase === 'channel' || phase === 'turn'
      if (phase.startsWith('ensemble_proposers')) {
        stream.setStreamActivity('Generating candidates')
      } else if (phase.startsWith('ensemble_aggregator')) {
        stream.setStreamActivity('Synthesizing candidates')
      } else if (!isPhaseAgnosticKeepalive) {
        stream.setStreamActivity('Planning next step')
      }
    } else if (!stream.streamBubble.value) {
      stream.showThinkingIndicator()
    }
  }

  function handleRpcCompaction(payload: CompactionPayload, meta: unknown) {
    if (isStaleEpoch(payload)) return
    if (bufferPendingStreamEvent('session.event.compaction', payload)) return
    const trackedPlacement = trackedLateCompactionPlacement(payload)
    if (!isCurrentTaskPayload(payload) && !trackedPlacement) return
    if (!acceptStreamSeq(payload)) return
    const safeMeta = (meta && typeof meta === 'object' ? meta : {}) as Record<string, unknown>
    const source = String(payload.source || '').toLowerCase()
    const status = compactionStatus(payload)
    const userVisible = payload.user_visible ?? payload.userVisible ?? true
    const taskId = payloadTaskId(payload)
    const ownedByCurrentTask = Boolean(
      taskId
      && activeStreamTaskId.value
      && taskId === activeStreamTaskId.value,
    )
    const canOwnActivity = source !== 'manual'
      && userVisible !== false
      && !['skipped', 'stale'].includes(status)
    const prefersActivity = canOwnActivity
      && (
        stream.isStreaming.value
        || safeMeta.authoritativeLive === true
        || ownedByCurrentTask
      )
    const settleCommittedActivity = trackedPlacement === 'activity'
      && !stream.isStreaming.value
    // An authoritative snapshot can contain compaction before any state/text
    // frame. Open the live reducer first so a later startStreaming() cannot
    // reset and discard the restored maintenance marker.
    if (prefersActivity && !stream.isStreaming.value && !settleCommittedActivity) {
      stream.startStreaming()
    }

    const requestedPlacement: ChatCompactionPlacement = trackedPlacement
      || (prefersActivity ? 'activity' : 'standalone')
    const presentation = options.showCompactionToast(payload || {}, {
      ...safeMeta,
      placement: requestedPlacement,
    })
    if (presentation === false) return
    const placement: ChatCompactionPlacement = presentation === 'activity'
      || presentation === 'standalone'
      ? presentation
      : requestedPlacement
    if (placement === 'activity') {
      if (settleCommittedActivity) {
        settleCommittedCompactionActivity(payload)
      } else {
        if (!stream.isStreaming.value && (safeMeta.authoritativeLive === true || ownedByCurrentTask)) {
          stream.startStreaming()
        }
        stream.recordCompactionActivity?.(payload)
      }
    }
    const compactionId = payloadCompactionId(payload)
    const durable = String(payload.durability || '').toLowerCase() === 'durable'
    if (source === 'manual' && status === 'completed' && (durable || compactionId)) {
      options.scheduleHistorySync()
    }
  }

  function handleRpcWarning(payload: WarningPayload) {
    if (isStaleEpoch(payload)) return
    if (!isCurrentSessionPayload(payload)) return
    // Silent compatibility warnings still own their stream sequence. Consuming
    // it before the display filter prevents a later replay from being accepted.
    if (!acceptStreamSeq(payload)) return
    if (
      payload.code === 'provider_reasoning_only_retry'
      || payload.code === 'provider_request_message_limit_recovery_success'
      || payload.code === 'context_auto_compaction_start'
      || payload.code === 'context_auto_compaction_retry'
    ) return
    // Let the view provide the locale-specific fallback when older gateways
    // omit a warning message.
    options.showWarningToast(String(payload.message || ''))
  }

  function handleRpcInputDisposition(payload: InputDispositionPayload) {
    if (isStaleEpoch(payload)) return
    if (!isCurrentSessionPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    // Primary sends also publish durable queued/applied disposition events.
    // Those events describe ingress ownership, not same-turn Steer UX. Older
    // gateways omitted intent for steer events, so only an explicit non-steer
    // intent is ignored for compatibility.
    if (payload.intent && payload.intent !== 'steer') return
    const disposition = payload.disposition
    if (!disposition) return
    const clientRequestId = String(payload.client_request_id || '')
    const clientMessageId = String(payload.client_message_id || '')
    const userMessageId = String(payload.user_message_id || '')
    const message = messages.value.find(candidate =>
      (clientMessageId && (
        candidate.clientId === clientMessageId
        || candidate.steerClientMessageId === clientMessageId
      ))
      || (clientRequestId && candidate.steerClientRequestId === clientRequestId)
      || (userMessageId && candidate.messageId === userMessageId),
    )
    const pendingIndex = pendingQueue.value.findIndex(candidate =>
      (clientRequestId && candidate.steerClientRequestId === clientRequestId)
      || (clientMessageId && candidate.steerClientMessageId === clientMessageId),
    )
    const pending = pendingIndex >= 0 ? pendingQueue.value[pendingIndex] : undefined
    const rawRevision = Number(payload.revision)
    const incomingRevision = Number.isInteger(rawRevision) && rawRevision >= 0
      ? rawRevision
      : undefined
    const currentRevision = message?.inputDispositionRevision
    const currentTerminal = message?.inputDisposition
      ? ['applied', 'promoted', 'cancelled', 'rejected'].includes(message.inputDisposition)
      : false
    const staleDisposition = Boolean(message) && (
      (
        currentRevision !== undefined
        && (
          incomingRevision === undefined
          || incomingRevision < currentRevision
        )
      )
      || (
        currentTerminal
        && disposition !== message?.inputDisposition
        && (
          incomingRevision === undefined
          || (
            currentRevision !== undefined
            && incomingRevision <= currentRevision
          )
        )
      )
    )
    if (staleDisposition) return
    const promotedTurnId = String(payload.promoted_turn_id || '').trim()
    const targetTurnId = String(
      promotedTurnId
      || payload.turn_id
      || payload.target_turn_id
      || payload.task_id
      || '',
    ).trim()
    if (message) {
      message.inputDisposition = disposition
      if (['applied', 'promoted', 'cancelled', 'rejected'].includes(disposition)) {
        message.steerStopRequested = false
      }
      if (incomingRevision !== undefined) {
        message.inputDispositionRevision = incomingRevision
      }
      if (userMessageId) message.messageId = userMessageId
      if (clientRequestId) message.steerClientRequestId = clientRequestId
      if (clientMessageId) message.steerClientMessageId = clientMessageId
      if (targetTurnId) message.turnId = targetTurnId
      if (disposition === 'promoted') {
        const promotedFromTurnId = String(
          payload.promoted_from_turn_id
          || payload.target_turn_id
          || payload.turn_id
          || '',
        ).trim()
        if (promotedFromTurnId) message.promotedFromTurnId = promotedFromTurnId
        messages.value = rehomePromotedSteerRows(messages.value)
      }
    }
    if (
      pendingIndex >= 0
      && ['applied', 'promoted', 'cancelled', 'rejected'].includes(disposition)
    ) {
      pendingQueue.value.splice(pendingIndex, 1)
    }
    const recovery = String(payload.recovery || '').trim().toLowerCase()
    const shouldRestore = disposition === 'cancelled'
      || (
        disposition === 'rejected'
        && (
          payload.retryable === true
          || recovery.includes('retry')
          || recovery.includes('resend')
          || recovery.includes('restore')
        )
      )
    const restoreKey = clientRequestId || clientMessageId || userMessageId
    if (shouldRestore && restoreKey) {
      const alreadyRestored = restoredSteerRequestIds.has(restoreKey)
        || message?.steerRestored === true
      if (!alreadyRestored) {
        const text = message?.text || pending?.text || ''
        if (text) options.restoreSteerIntoComposer?.(text)
        restoredSteerRequestIds.add(restoreKey)
        if (message) message.steerRestored = true
      }
    }
    options.scheduleHistorySync()
  }

  function messageAlreadyPresent(candidate: ChatMessage): boolean {
    if (candidate.messageId) {
      return messages.value.some(message => message.messageId === candidate.messageId)
    }
    return messages.value.some(message =>
      message.role === candidate.role
      && message.text === candidate.text
      && message.provenanceKind === candidate.provenanceKind
      && message.provenanceSourceSessionKey === candidate.provenanceSourceSessionKey
      && message.provenanceSourceTool === candidate.provenanceSourceTool
      && (candidate.provenanceSourceTool === 'subagent_completion' || message.ts === candidate.ts))
  }

  function appendDurableEventMessage(message: ChatMessage) {
    if (messageAlreadyPresent(message)) return
    messages.value.push(message)
    // The pushed row is deliberately immediate. History remains authoritative
    // for ordering and durable metadata and will replace/dedupe by message id.
    options.scheduleHistorySync()
  }

  function handleRpcCronResult(payload: CronResultPayload) {
    if (isStaleEpoch(payload)) return
    if (!isCurrentSessionPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    const message = payload.message
    if (!message || typeof message !== 'object') return
    const text = typeof message.text === 'string' ? message.text : ''
    if (!text) return
    appendDurableEventMessage({
      role: typeof message.role === 'string' && message.role ? message.role : 'assistant',
      text,
      ts: message.timestamp ?? new Date().toISOString(),
      messageId: String(message.messageId || message.message_id || '') || undefined,
      provenanceKind: String(message.provenanceKind || 'cron'),
      provenanceSourceTool: String(message.provenanceSourceTool || ''),
      provenanceSourceSessionKey: String(message.provenanceSourceSessionKey || ''),
    })
  }

  function handleRpcSubagentCompletion(payload: SubagentCompletionPayload) {
    if (isStaleEpoch(payload)) return
    if (!isCurrentSessionPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    const durablePayload = { ...payload }
    delete durablePayload.session_key
    delete durablePayload.sessionKey
    delete durablePayload.stream_seq
    delete durablePayload.epoch
    // message_id correlates this push with the separately persisted transcript
    // row. It is transport metadata, not part of the subagent business payload
    // stored/rendered as JSON.
    delete durablePayload.message_id
    delete durablePayload.messageId
    const sourceSessionKey = String(payload.child_session_key || '')
    appendDurableEventMessage({
      role: 'system',
      text: JSON.stringify(durablePayload),
      ts: new Date().toISOString(),
      messageId: String(payload.message_id || payload.messageId || '') || undefined,
      provenanceKind: 'internal_system',
      provenanceSourceSessionKey: sourceSessionKey,
      provenanceSourceTool: 'subagent_completion',
    })
  }

  function handleRpcEpochChanged(payload: SessionEventPayload) {
    const ep = payload?.epoch
    if (typeof ep === 'number' && Number.isFinite(ep) && ep > currentEpoch.value) {
      activeTaskGroups.value.clear()
      currentEpoch.value = ep
    }
  }

  function handleRpcSessionsChanged(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!isCurrentSessionPayload(payload)) return
    if (isStoppedTerminalSessionChange(payload)) {
      syncTerminalSessionChange(payload)
      return
    }
    if (
      sessionChangeIsTerminal(payload) &&
      bufferPendingTerminalEvent({ kind: 'session-change', payload })
    ) return
    if (!isCurrentTaskPayload(payload)) return
    if (sessionChangeIsTerminal(payload)) {
      // `occasion: 'turn_end'` / `run_status: 'idle'` are a *fallback* terminal
      // (see sessionChangeIsTurnEndSignal): the primary completion path remains
      // `session.event.done`. A bare idle frame is ambiguous — it can be a stale
      // snapshot replayed while our own `chat.send` is still pending, or an idle
      // heartbeat racing an approval prompt. Only collapse the live turn once it
      // is bound to a concrete task (not the PENDING window) and no interrupt is
      // on screen; otherwise keep streaming and let `session.event.done` or the
      // idle watchdog decide.
      const turnEndFallback = !sessionChangeIsStrongTerminal(payload)
      if (
        turnEndFallback
        && (
          activeStreamTaskId.value === PENDING_STREAM_TASK_ID
          || options.hasActiveInterrupt?.value === true
        )
      ) {
        return
      }
      markTaskSettled(payload)
      const state = options.sessionRunStatus(payload)
      const interrupted = state.status === 'cancelled' || state.status === 'interrupted'
      if (activeTaskGroups.value.size > 0 && !interrupted) {
        if (stream.isStreaming.value) stream.endStreaming()
        options.applySessionRunState(activeTaskGroupRunState(payload))
        options.scheduleHistorySync()
        return
      }
      syncTerminalSessionChange(payload)
      return
    }
    const carriesRunState = (
      'run_status' in payload
      || 'runStatus' in payload
      || 'active_task' in payload
      || 'activeTask' in payload
      || 'last_task' in payload
      || 'lastTask' in payload
    )
    // Recents-only changes (for example an asynchronously generated title)
    // share the sessions.changed event name but carry no task state. They must
    // not collapse a live task snapshot or discard its steer capability.
    if (!carriesRunState) return
    options.applySessionRunState(payload)
  }

  function handleRpcTaskQueued(payload: SessionEventPayload) {
    if (!isCurrentSessionPayload(payload)) return
    // queued can describe another same-session task. Keep the fresh send on
    // PENDING until its chat.send response supplies the accepted task id.
    options.applySessionRunState({ run_status: 'queued', active_task: { ...(payload || {}), status: 'queued' } })
  }

  function handleRpcTaskRunning(payload: SessionEventPayload) {
    if (!isCurrentSessionPayload(payload)) return
    // task.running is the authoritative "this task now owns the live stream"
    // signal once no chat.send response is pending. While PENDING, another
    // same-session task may start first; only the response can identify which
    // task belongs to this optimistic stream.
    const taskId = payloadTaskId(payload)
    if (taskId && activeStreamTaskId.value !== PENDING_STREAM_TASK_ID) {
      activeStreamTaskId.value = taskId
    }
    options.applySessionRunState({ run_status: 'running', active_task: { ...(payload || {}), status: 'running' } })
  }

  function handleRpcTaskGroupWaiting(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!acceptStreamSeq(payload)) return
    noteTaskGroupActive(payload)
  }

  function handleRpcTaskGroupSynthesizing(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!acceptStreamSeq(payload)) return
    noteTaskGroupActive(payload)
  }

  function handleRpcTaskGroupDone(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!acceptStreamSeq(payload)) return
    noteTaskGroupTerminal(payload, 'succeeded')
  }

  function handleRpcTaskGroupFailed(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (!acceptStreamSeq(payload)) return
    noteTaskGroupTerminal(payload, 'failed')
  }

  function handleRpcRouterDecision(payload: RouterDecisionPayload) {
    if (isStaleEpoch(payload)) return
    if (bufferPendingStreamEvent('session.event.router_decision', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    options.queueRouterDecision(payload)
  }

  function handleRpcEnsembleProgress(payload: EnsembleProgressPayload) {
    if (isStaleEpoch(payload)) return
    if (bufferPendingStreamEvent('session.event.ensemble_progress', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    restartStreamingForLiveEvent()
    // A lifecycle frame is a real gateway event. Keep the hard no-event timer
    // aligned with the wire even when heartbeat cadence is slower than progress.
    stream.resetStreamIdleTimer()
    options.appendEnsembleProgress(payload)
  }

  function handleRpcRouterControlReplay(payload: SessionEventPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (bufferPendingStreamEvent('session.event.router_control_replay', payload)) return
    if (!isCurrentTaskPayload(payload)) return
    if (!acceptStreamSeq(payload)) return
    options.handleRouterControlReplay()
  }

  function handleRpcAny(
    rawEvent: string,
    rawPayload: unknown,
    meta?: Record<string, unknown>,
  ) {
    const payloadObj = (rawPayload && typeof rawPayload === 'object' ? rawPayload : {}) as SessionEventPayload
    const taskSucceededFallback = rawEvent === 'task.succeeded'
    const terminalStatus = eventTaskTerminalStatus(rawEvent)
    const rawStatus = payloadObj.run_status || payloadObj.runStatus || payloadObj.status || ''
    const normalizedStatus = options.normalizeRunStatus(String(rawStatus))
    if (
      normalizedStatus === 'approval_pending' ||
      (typeof rawEvent === 'string' && rawEvent.includes('approval') && isCurrentSessionPayload(payloadObj))
    ) {
      if (!isCurrentSessionPayload(payloadObj)) return
      options.applySessionRunState({
        run_status: 'approval_pending',
        active_task: { ...(payloadObj || {}), status: 'approval_pending' },
      })
      return
    }
    if (
      isTerminalEvent(rawEvent) &&
      bufferPendingTerminalEvent({ kind: 'event', event: rawEvent, payload: payloadObj })
    ) return
    if (
      rawEvent === 'session.event.thinking' &&
      bufferPendingStreamEvent(rawEvent, payloadObj)
    ) return
    // A stale task's terminal/done/error must not end the current turn's stream
    // or push its "Turn failed" into the live transcript (issue #344). Approvals
    // above stay ungated; everything below mutates the current turn.
    if (!isStoppedCancelledTerminalEvent(terminalStatus, payloadObj) && !isCurrentTaskPayload(payloadObj)) return
    if (terminalStatus) {
      if (!isCurrentSessionPayload(payloadObj)) return
      const terminalRunStatus = terminalStatus === 'succeeded' ? 'idle' : terminalStatus === 'abandoned' ? 'interrupted' : terminalStatus
      if (activeTaskGroups.value.size > 0) {
        options.applySessionRunState(activeTaskGroupRunState(payloadObj))
      } else {
        const terminalRunState = { run_status: terminalRunStatus, last_task: { ...(payloadObj || {}), status: terminalStatus } }
        options.applySessionRunState(terminalRunState)
      }
    }

    const normalized = normalizeTaskTerminalEvent(rawEvent, payloadObj)
    if (normalized && isStaleEpoch(payloadObj)) return
    if (normalized && !stream.isStreaming.value) {
      markTaskSettled(payloadObj)
      activeStreamTaskId.value = FINISHED_STREAM_TASK_ID
      return
    }

    const event = normalized ? normalized.event : rawEvent
    const payload = normalized ? normalized.payload : payloadObj

    if (typeof event !== 'string') return
    if (event.startsWith('session.event.') && isStaleEpoch(payload)) return
    if (!acceptStreamSeq(payload)) return
    if (event.startsWith('session.event.task_group.')) return
    if (event === 'sessions.changed') return

    if (event === 'session.event.thinking') {
      if (aborted.value) return
      // 编排器自己的决策叙述（`kind: "narration"`）不是模型推理：不进入思考区，
      // 否则用户看到的「思考」是路由说明而不是模型真实思维链。
      if (String((payload as Record<string, unknown>)?.kind || '') === 'narration') return
      const thinkingText = (payload as SessionEventPayload).text
      if (typeof thinkingText !== 'string' || !thinkingText) return
      stream.resetStreamIdleTimer()
      appendThinkingDelta(thinkingText, payload)
      return
    }

    // v3.3 插话生命周期事件（Rust steer_v2 发布）：
    // - steer_accepted：插话已入队，等待运行边界应用 → 用户气泡标记「已并入运行」
    // - steers_applied：turn 结束时插话已并入回复 → 标记「已应用」并提示查看结论
    if (event === 'session.event.status') {
      const kind = String((payload as Record<string, unknown>)?.kind || '')
      if (kind === 'steer_accepted' || kind === 'steers_applied') {
        const clientMessageId = String(
          (payload as Record<string, unknown>)?.client_message_id || '',
        )
        const target = clientMessageId
          ? messages.value.find(m => m.steerClientMessageId === clientMessageId)
          : undefined
        if (target && (target.inputDisposition === 'steering' || kind === 'steers_applied')) {
          target.inputDisposition = kind === 'steer_accepted' ? 'steering' : 'applied'
        }
        stream.resetStreamIdleTimer()
        return
      }
    }

    // 每次真实 LLM 调用后网关都会发 `session.event.usage`（core usage::record）。
    // 这是**唯一**带真实 token 的帧：`session.event.done` 的载荷里没有用量字段
    // （实测 done = {text, reason, strategy, completeness}），此前只从 done 累加
    // 的 `usageAccum` 因此永远是 0 —— 底部计量条显示的 token 也就是假的。
    if (event === 'session.event.usage') {
      // 只累计「本会话」的用量。归属判据有两条，先强后弱：
      //
      // ① **帧信封的 meta.session（权威）**：网关 `envelope_to_frame` 给每一帧都盖了
      //    `meta.session`，用量帧的 payload 里**没有**会话字段。此前直接丢掉 meta、
      //    只靠「本页是否在流式中」判归属 —— 而多标签页并行时，A 页在跑模型、B 页刚
      //    发出请求同样处于 `isStreaming`，于是 A 的 usage 帧被广播到 B 页并被 B 计入，
      //    表现为「B 轮次 0 / 首 token 从未收到，页脚却显示 token 108854」（真机审计
      //    2026-10-29 P1-1）。meta 带着会话键时以它为准，不是本会话就丢弃。
      // ② 没有 meta（旧网关 / 重放路径）时保留原判据：非流式期间到达的 usage 属于
      //    定时任务或其它会话，不计入。
      const frameSession = frameSessionKey(payload, meta)
      if (frameSession && sessionKey.value && frameSession !== sessionKey.value) return
      if (!stream.isStreaming.value) return
      const u = (payload as Record<string, unknown>) || {}
      const input = Number(u.input_tokens ?? u.inputTokens ?? u.prompt_tokens ?? u.promptTokens ?? 0) || 0
      const output = Number(u.output_tokens ?? u.outputTokens ?? u.completion_tokens ?? u.completionTokens ?? 0) || 0
      const cached = Number(u.cached_tokens ?? u.cachedTokens ?? u.cache_read_tokens ?? u.cacheReadTokens ?? 0) || 0
      if (input || output || cached) {
        usageAccum.value.input += input
        usageAccum.value.output += output
        usageAccum.value.cacheRead += cached
      }
      if (typeof u.model === 'string' && u.model) usageModel.value = u.model
      return
    }

    if (event.endsWith('.done') || event === 'chat.done') {
      markTaskSettled(payload)
      const donePayload = payload as ChatDoneUsagePayload
      const u = donePayload.usage || donePayload || {}
      if (u.input_tokens || u.output_tokens) {
        usageAccum.value.input += u.input_tokens || 0
        usageAccum.value.output += u.output_tokens || 0
        usageAccum.value.cacheRead += u.cached_tokens || 0
        usageAccum.value.cacheWrite += u.cache_write || 0
        if (u.cost_usd != null) usageAccum.value.cost = (usageAccum.value.cost || 0) + u.cost_usd
      }
      if (u.model) usageModel.value = u.model
      options.saveWidgetState()

      stream.reconcileFinalText(doneTextSnapshot(donePayload, u))

      if (payload?.reason === 'aborted') {
        options.clearPendingRouterDecision()
      } else {
        options.flushPendingRouterDecision()
      }
      // Done backfills the turn's reasoning: prefer the authoritative
      // reasoning_content, fall back to accumulated live thinking deltas.
      const rawReasoningContent = (payload as SessionEventPayload).reasoning_content
      const doneReasoning = typeof rawReasoningContent === 'string'
        ? rawReasoningContent.trim()
        : ''
      const liveThinking = streamThinking.value
      const reasoningText = doneReasoning || liveThinking?.text.trim() || ''
      const reasoningSeconds = (() => {
        if (!liveThinking) return 0
        const now = Date.now()
        const serverStartedAt = liveThinking.serverStartedAt
        let elapsedMs = now - liveThinking.startedAt
        if (serverStartedAt != null) {
          const serverDoneAt = trustedReasoningDoneAt(
            (payload as SessionEventPayload).emitted_at,
            serverStartedAt,
            now,
          )
          if (serverDoneAt != null) elapsedMs = serverDoneAt - serverStartedAt
        }
        return Math.max(0, Math.floor(elapsedMs / 1000))
      })()
      clearLiveThinking()
      const messageCountBeforeEnd = messages.value.length
      stream.endStreaming()
      // endStreaming pushes the assistant message only when the turn kept
      // visible output; sentinel/empty bubbles must not record reasoning.
      // Bind reasoning to that exact bubble, then keep a record so the
      // measured duration survives history replacements.
      const completedMessage = messages.value[messageCountBeforeEnd]
      const completedAssistant = completedMessage?.role === 'assistant'
        ? completedMessage
        : null
      if (completedAssistant && payload?.reason !== 'aborted') {
        // task.succeeded is a lifecycle-only fallback when the richer done
        // receipt went missing; do not mislabel its task metadata as usage.
        if (!taskSucceededFallback) completedAssistant.usage = doneUsagePayload(donePayload)
        if (u.model) completedAssistant.model = u.model
        if (u.input_tokens) completedAssistant.input_tokens = u.input_tokens
        if (u.output_tokens) completedAssistant.output_tokens = u.output_tokens
      }
      if (reasoningText && payload?.reason !== 'aborted' && completedAssistant) {
        completedAssistant.reasoning = { text: reasoningText, seconds: reasoningSeconds }
        recordTurnReasoning(reasoningText, reasoningSeconds, completedAssistant.text.trim())
      }
      options.scheduleHistorySync()

      if (payload?.reason === 'aborted') {
        options.popAllPendingIntoComposer()
        const cancelledRunState = { run_status: 'cancelled', last_task: { ...(payload || {}), status: 'cancelled' } }
        options.applySessionRunState(cancelledRunState)
      } else if (activeTaskGroups.value.size > 0) {
        options.applySessionRunState(activeTaskGroupRunState({ reason: 'task_group_active' }))
      } else {
        options.applySessionRunState({ run_status: 'idle', last_task: { status: 'succeeded' } })
      }

      if (pendingQueue.value.length > 0 && payload?.reason !== 'aborted') {
        options.schedulePendingDrainAfterTerminal()
      }
      activeStreamTaskId.value = FINISHED_STREAM_TASK_ID
    } else if (event.endsWith('.error')) {
      markTaskSettled(payload)
      options.clearPendingRouterDecision()
      clearLiveThinking()
      stream.endStreaming()
      const rawErrorCode = (payload as { code?: unknown })?.code
      const errorCode = typeof rawErrorCode === 'string' ? rawErrorCode : undefined
      const serverMessage = eventSessionErrorMessage(payload)
      messages.value.push({
        role: 'error',
        // R65/B1：结构化错误帧（如计划步骤失败）携带 step_id / step_title，
        // 按码本地化时用其插值，避免回显后端内部术语。
        text: localizedChatErrorMessage(errorCode, serverMessage, payload ?? {}),
        errorCode,
        terminalNotice: true,
        ts: new Date().toISOString(),
      })
      options.scheduleHistorySync()
      if (activeTaskGroups.value.size > 0) {
        options.applySessionRunState(activeTaskGroupRunState(payload))
      } else {
        options.applySessionRunState({ run_status: 'failed', last_task: { ...(payload || {}), status: 'failed' } })
      }
      activeStreamTaskId.value = FINISHED_STREAM_TASK_ID
    }
  }

  let connectionLostNoted = false
  let connectionLostNotice: ChatMessage | null = null
  let connectionStateGeneration = 0

  function clearConnectionLostStatus() {
    connectionLostNoted = false
    if (!connectionLostNotice) return
    const noticeIndex = messages.value.indexOf(connectionLostNotice)
    if (noticeIndex >= 0) messages.value.splice(noticeIndex, 1)
    connectionLostNotice = null
  }

  function handleRpcConnectionState(state: string) {
    const stateGeneration = ++connectionStateGeneration
    const recovery = options.handleSessionConnectionState?.(state)
    if (state === 'connected') {
      clearConnectionLostStatus()
      stream.hideThinkingIndicator()
      const subscription = recovery?.live ?? options.subscribeSession?.()
      void Promise.resolve(subscription)
        .then((subscribed) => {
          if (isAuthoritativeSessionSubscription(subscribed)) {
            return options.onSessionSubscribed?.()
          }
        })
        .catch((error: unknown) => {
          console.warn(
            'Session recovery after reconnect failed:',
            error instanceof Error ? error.message : error,
          )
        })
      if (sessionKey.value) {
        const connectedSessionKey = sessionKey.value
        // Preserve critical frame ordering after reconnect without waiting for a
        // potentially slow history response before refreshing independent UI.
        const criticalRequestsQueued = recovery?.criticalRequestsQueued
          ?? Promise.resolve()
        void criticalRequestsQueued.then(() => {
          if (
            connectionStateGeneration === stateGeneration
            && sessionKey.value === connectedSessionKey
          ) {
            options.loadCurrentSessionUsage()
            void options.refreshRunModePreference?.()
          }
        })
      }
      if (!recovery) {
        options.loadCurrentSessionUsage()
        options.loadHistory?.()
      }
      if (stream.isStreaming.value) stream.resetStreamIdleTimer()
    }
    if (state === 'disconnected' && stream.isStreaming.value) {
      // Keep the idle watchdog armed so a run whose events never resume still
      // times out honestly. The row is transient and removed after reconnect.
      stream.showThinkingIndicator()
      if (!connectionLostNoted) {
        connectionLostNoted = true
        connectionLostNotice = {
          role: 'system',
          text: 'Connection lost — trying to reconnect…',
          ts: new Date().toISOString(),
        }
        messages.value.push(connectionLostNotice)
      }
    }
  }

  /**
   * T04：跨项目记忆召回（Rust 仅在有命中时发布）。按 session 过滤后，向当前 live
   * 活动时间线插入一条轻量信息行。**不**触碰 idle 计时、不改变流状态——它只是
   * 一条可观测性记录，流式相位仍由内容事件驱动。
   */
  function handleRpcMemoryRecall(payload: MemoryRecallPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (!payloadIsCurrentSession(payload, sessionKey.value)) return
    const hits = Number(payload.hits ?? 0)
    if (!Number.isFinite(hits) || hits <= 0) return
    if (!stream.useReducer.value) return
    stream.appendFrame({
      kind: 'status',
      action: 'memory_recall',
      label: '',
      at: Date.now(),
      detail: String(Math.floor(hits)),
    })
  }

  /** agent 工具回路轮次 / 交付流水线步骤：可读 token（轮次或 `i/total`）。 */
  function stepLoopToken(payload: StepLoopPayload): string {
    const round = Number(payload.round)
    if (Number.isFinite(round) && round > 0) return String(Math.floor(round))
    const index = Number(payload.index)
    const total = Number(payload.total)
    if (Number.isFinite(index) && Number.isFinite(total) && total > 0) {
      return `${Math.floor(index) + 1}/${Math.floor(total)}`
    }
    return ''
  }

  function handleRpcStepStart(payload: StepLoopPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (!payloadIsCurrentSession(payload, sessionKey.value)) return
    const token = stepLoopToken(payload)
    if (!token) return
    if (!stream.useReducer.value) return
    stream.appendFrame({
      kind: 'status',
      action: 'step_start',
      label: '',
      at: Date.now(),
      detail: token,
    })
  }

  function handleRpcStepFinish(payload: StepLoopPayload) {
    if (isStaleEpoch(payload)) return
    if (aborted.value) return
    if (!payloadIsCurrentSession(payload, sessionKey.value)) return
    const token = stepLoopToken(payload)
    if (!token) return
    if (!stream.useReducer.value) return
    // reason 优先取工具回路 reason，其次流水线 status；均为我方稳定 token。
    const reason = String(payload.reason || payload.status || '')
    stream.appendFrame({
      kind: 'status',
      action: 'step_finish',
      label: '',
      at: Date.now(),
      detail: `${token}|${reason}`,
    })
  }

  const handlers: ChatRpcSubscriptionHandlers = {
    onTextDelta: handleRpcTextDelta,
    onToolUseStart: handleRpcToolUseStart,
    onToolUseDelta: handleRpcToolUseDelta,
    onToolResult: handleRpcToolResult,
    onArtifact: handleRpcArtifact,
    onArtifactUpdated: handleRpcArtifactUpdated,
    onMemoryRecall: handleRpcMemoryRecall,
    onStepStart: handleRpcStepStart,
    onStepFinish: handleRpcStepFinish,
    onStateChange: handleRpcStateChange,
    onRunHeartbeat: handleRpcRunHeartbeat,
    onCompaction: handleRpcCompaction,
    onWarning: handleRpcWarning,
    onInputDisposition: handleRpcInputDisposition,
    onCronResult: handleRpcCronResult,
    onSubagentCompletion: handleRpcSubagentCompletion,
    onEpochChanged: handleRpcEpochChanged,
    onSessionsChanged: handleRpcSessionsChanged,
    onTaskQueued: handleRpcTaskQueued,
    onTaskRunning: handleRpcTaskRunning,
    onTaskGroupWaiting: handleRpcTaskGroupWaiting,
    onTaskGroupSynthesizing: handleRpcTaskGroupSynthesizing,
    onTaskGroupDone: handleRpcTaskGroupDone,
    onTaskGroupFailed: handleRpcTaskGroupFailed,
    onRouterDecision: handleRpcRouterDecision,
    onEnsembleProgress: handleRpcEnsembleProgress,
    onRouterControlReplay: handleRpcRouterControlReplay,
    onAny: handleRpcAny,
    onConnectionState: handleRpcConnectionState,
  }

  return {
    handlers,
    bindActiveStreamTask,
    restoreLiveTurnSnapshot,
    streamThinkingText,
    streamThinkingElapsedText,
    attachTurnReasoning,
  }
}
