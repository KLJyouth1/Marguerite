// N1 回归：直播轮的「正在工作 · Ns」必须走**单一回合时钟**——相位切换时不得
// 归零，否则会与旁边「思考中 · Ns」自相矛盾（真机截图：工作 0s / 思考 63s）。
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { ref } from 'vue'
import { useChatStream } from './useChatStream'
import type { ChatMessage, ChatRunStatus } from '@/types/chat'
import type { InterruptViewState } from '@/types/parts'

function makeStream() {
  return useChatStream({
    messages: ref<ChatMessage[]>([]),
    lastHeaderRole: ref(''),
    aborted: ref(false),
    autoScroll: ref(true),
    runStatus: ref<ChatRunStatus>({ status: 'idle', label: '', task: null }),
    applySessionRunState: vi.fn(),
    renderMarkdown: vi.fn((t: string) => `<p>${t}</p>`) as never,
    stripDirectiveTags: (t: string) => t,
    stripGeneratedArtifactMarkers: (t: string) => t,
    scrollToBottom: vi.fn(),
    rpcPolicy: undefined,
    interruptState: ref<ReadonlyMap<string, InterruptViewState>>(new Map()),
  })
}

describe('live turn clock (N1)', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', () => 1)
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('does not reset the elapsed chip when the activity phase changes mid-turn', () => {
    const api = makeStream()
    api.startStreaming()
    expect(api.streamPhaseElapsed.value).toBe('0s')
    expect(api.streamTurnElapsedSeconds.value).toBe(0)

    // Keep the signal fresh (below STALE_SIGNAL_MS = 20s) so the assertion
    // exercises the normal path, not the stall readout.
    vi.advanceTimersByTime(10_000)
    expect(api.streamPhaseElapsed.value).toBe('10s')
    expect(api.streamTurnElapsedSeconds.value).toBe(10)

    // A phase change (e.g. a tool result arriving) must NOT zero the counter —
    // with the old per-phase clock this read "10s" again (the contradiction
    // with a sibling "Thinking · Ns").
    api.setStreamActivity('Planning next step', 'plan:1')
    vi.advanceTimersByTime(10_000)
    expect(api.streamPhaseElapsed.value).toBe('20s')
    expect(api.streamTurnElapsedSeconds.value).toBe(20)
    api.cleanup()
  })

  it('re-arms the turn clock on a new turn', () => {
    const api = makeStream()
    api.startStreaming()
    vi.advanceTimersByTime(30_000)
    expect(api.streamTurnElapsedSeconds.value).toBe(30)

    api.startStreaming()
    expect(api.streamTurnElapsedSeconds.value).toBe(0)
    api.cleanup()
  })
})