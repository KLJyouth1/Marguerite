// T04：补订此前被前端丢弃的 3 类 Rust 事件（memory_recall / step_start / step_finish）。
// 两个层次都验：① 投影把新 action 映射为专有文案（真实 projectAssistantActivityTimeline）；
// ② 事件处理器按 session 过滤后向 live 活动时间线追加帧（真实 useChatRpcEventHandlers）。
import { describe, expect, it, vi } from 'vitest'
import { effectScope, ref } from 'vue'
import { useChatRpcEventHandlers, type ChatRpcStreamApi } from './useChatRpcEventHandlers'
import type { ChatPendingItem, ChatRunStatus } from '@/types/chat'
import type { StatusPart } from '@/types/parts'
import { projectAssistantActivityTimeline } from '@/utils/chat/assistantActivity'

const SESSION = 'agent:main:test'

function statusPart(partial: Partial<StatusPart>): StatusPart {
  return { action: '', label: '', at: 0, ...partial }
}

describe('T04 projection labels', () => {
  it('maps memory_recall / step_start / step_finish to dedicated codes with structured params', () => {
    const projection = projectAssistantActivityTimeline([], {
      statusHistory: [
        statusPart({ action: 'memory_recall', at: 1, detail: '3' }),
        statusPart({ action: 'step_start', at: 2, detail: '2' }),
        statusPart({ action: 'step_finish', at: 3, detail: '2|completed' }),
      ],
    })
    const codes = projection.statusSteps.map(step => step.label)
    expect(codes).toContainEqual({ code: 'chat.activity.memoryRecall', params: { count: 3 } })
    expect(codes).toContainEqual({ code: 'chat.activity.stepRound', params: { round: '2' } })
    expect(codes).toContainEqual({
      code: 'chat.activity.stepRoundDone',
      params: { round: '2', reason: 'completed' },
    })
  })
})

function harness() {
  const messages = ref([])
  const sessionKey = ref(SESSION)
  const stream: ChatRpcStreamApi = {
    isStreaming: ref(true),
    streamBubble: ref(true),
    streamHasVisibleOutput: ref(false),
    startStreaming: vi.fn(),
    endStreaming: vi.fn(),
    appendDelta: vi.fn(),
    scheduleRender: vi.fn(),
    appendToolCall: vi.fn(),
    appendToolDelta: vi.fn(),
    appendToolResult: vi.fn(),
    appendArtifact: vi.fn(),
    reconcileFinalText: vi.fn(),
    resetLiveTurnState: vi.fn(),
    resetStreamIdleTimer: vi.fn(),
    clearStreamIdleTimer: vi.fn(),
    setStreamActivity: vi.fn(),
    recordCompactionActivity: vi.fn(),
    showThinkingIndicator: vi.fn(),
    hideThinkingIndicator: vi.fn(),
    appendFrame: vi.fn(),
    setThinkingContent: vi.fn(),
    // 真实生产默认 ON：帧必须被真正追加。
    useReducer: ref(true),
  }
  const usageAccum = ref({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: null, routedTurns: 0, sessionSaved: 0 })
  const scope = effectScope()
  const api = scope.run(() => useChatRpcEventHandlers({
    sessionKey,
    currentEpoch: ref(0),
    lastStreamSeq: ref(0),
    activeTaskGroups: ref(new Set<string>()),
    activeStreamTaskId: ref(''),
    aborted: ref(false),
    messages,
    pendingQueue: ref<ChatPendingItem[]>([]),
    usageAccum,
    usageModel: ref(''),
    stream,
    normalizeRunStatus: (s: string) => s,
    sessionRunStatus: (): ChatRunStatus => ({ status: 'idle', label: 'Idle', task: null }),
    applySessionRunState: vi.fn(),
    queueRouterDecision: vi.fn(),
    appendEnsembleProgress: vi.fn(),
    markEnsembleHandoff: vi.fn(),
    flushPendingRouterDecision: vi.fn(),
    clearPendingRouterDecision: vi.fn(),
    handleRouterControlReplay: vi.fn(),
    showCompactionToast: vi.fn(),
    showWarningToast: vi.fn(),
    scheduleHistorySync: vi.fn(),
    schedulePendingDrainAfterTerminal: vi.fn(),
    popAllPendingIntoComposer: vi.fn(() => false),
    restoreSteerIntoComposer: vi.fn(),
    saveWidgetState: vi.fn(),
    subscribeSession: vi.fn(),
    onSessionSubscribed: vi.fn(),
    loadHistory: vi.fn(),
    handleSessionConnectionState: vi.fn(),
    loadCurrentSessionUsage: vi.fn(),
    refreshRunModePreference: vi.fn(),
  }))!
  return { api, stream, sessionKey, usageAccum }
}

describe('session.event.usage → 真实 token 累加', () => {
  it('累加 snake_case 的 prompt/completion tokens（网关真实形状）', () => {
    const { api, usageAccum } = harness()
    api.handlers.onAny('session.event.usage', {
      model: 'deepseek-v4-flash',
      prompt_tokens: 883,
      completion_tokens: 2,
      total_tokens: 885,
    } as never)
    expect(usageAccum.value.input).toBe(883)
    expect(usageAccum.value.output).toBe(2)
  })

  it('非流式期间到达的 usage（定时任务/其它会话）不计入本会话', () => {
    const { api, stream, usageAccum } = harness()
    stream.isStreaming.value = false
    api.handlers.onAny('session.event.usage', { prompt_tokens: 100, completion_tokens: 5 } as never)
    expect(usageAccum.value.input).toBe(0)
    expect(usageAccum.value.output).toBe(0)
  })

  it('缓存 token 计入 cacheRead，供命中率使用', () => {
    const { api, usageAccum } = harness()
    api.handlers.onAny('session.event.usage', {
      input_tokens: 1000,
      output_tokens: 50,
      cached_tokens: 400,
    } as never)
    expect(usageAccum.value.cacheRead).toBe(400)
  })
})

describe('T04 handlers append activity rows', () => {
  it('appends a memory_recall row for a matching session with hits>0', () => {
    const { api, stream } = harness()
    api.handlers.onMemoryRecall({ sessionKey: SESSION, hits: 3 } as never)
    expect(stream.appendFrame).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'status', action: 'memory_recall', detail: '3',
    }))
  })

  it('drops memory_recall with zero hits or another session', () => {
    const { api, stream } = harness()
    api.handlers.onMemoryRecall({ sessionKey: SESSION, hits: 0 } as never)
    api.handlers.onMemoryRecall({ sessionKey: 'other', hits: 5 } as never)
    expect(stream.appendFrame).not.toHaveBeenCalled()
  })

  it('appends step_start / step_finish rows carrying the round token + reason', () => {
    const { api, stream } = harness()
    api.handlers.onStepStart({ sessionKey: SESSION, round: 2 } as never)
    expect(stream.appendFrame).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'status', action: 'step_start', detail: '2',
    }))
    api.handlers.onStepFinish({ sessionKey: SESSION, round: 2, reason: 'completed' } as never)
    expect(stream.appendFrame).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'status', action: 'step_finish', detail: '2|completed',
    }))
  })

  it('uses the pipeline index/total token when round is absent', () => {
    const { api, stream } = harness()
    api.handlers.onStepStart({ sessionKey: SESSION, index: 1, total: 4 } as never)
    expect(stream.appendFrame).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'status', action: 'step_start', detail: '2/4',
    }))
  })
})