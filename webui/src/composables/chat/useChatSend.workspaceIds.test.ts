import { describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { resolveSendWorkspaceIds, useChatSend, type UseChatSendOptions } from './useChatSend'
import type { FoldLiveTurnMode } from './useChatTurnLog'
import type { Attachment, ChatMessage } from '@/types/chat'
import type { CollaborationMode } from '@/types/plans'
import type { BusySendMode } from '@/composables/chat/useChatPendingQueue'

const pushToast = vi.hoisted(() => vi.fn())

vi.mock('@/composables/useToasts', () => ({
  useToasts: () => ({ pushToast }),
}))

function makeOptions(overrides: Partial<UseChatSendOptions> = {}) {
  const rpc = {
    call: vi.fn().mockResolvedValue({ sessionKey: 'agent:main:webchat:test' }),
  }
  const stream: UseChatSendOptions['stream'] = {
    isStreaming: ref(false),
    streamBubble: ref(false),
    streamHasVisibleOutput: ref(false),
    startStreaming: vi.fn(),
    endStreaming: vi.fn(),
    checkpointForUserMessage: vi.fn(),
    appendDelta: vi.fn(),
    scheduleRender: vi.fn(),
    appendToolCall: vi.fn(),
    appendToolDelta: vi.fn(),
    appendToolResult: vi.fn(),
    appendArtifact: vi.fn(),
    reconcileFinalText: vi.fn(),
    resetStreamIdleTimer: vi.fn(),
    clearStreamIdleTimer: vi.fn(),
    setStreamActivity: vi.fn(),
    showThinkingIndicator: vi.fn(),
    hideThinkingIndicator: vi.fn(),
    appendFrame: vi.fn(),
    setThinkingContent: vi.fn(),
    useReducer: ref<FoldLiveTurnMode>(false),
  }
  const options: UseChatSendOptions = {
    rpc,
    inputText: ref('hello'),
    messages: ref<ChatMessage[]>([]),
    sessionKey: ref('agent:main:webchat:test'),
    pendingQueueOwnerContext: ref(null),
    busySendMode: ref<BusySendMode>('queue'),
    modelRoutingMode: ref<'off'>('off'),
    modelRoutingSettingsBusy: ref(false),
    elevatedMode: ref(''),
    runMode: ref('safe'),
    pendingAttachments: ref<Attachment[]>([]),
    pendingSessionIntent: ref('followup'),
    initialCollaborationMode: ref<CollaborationMode>('default'),
    pendingForkBeforeMessageId: ref(null),
    aborted: ref(false),
    activeStreamTaskId: ref(''),
    activeStreamSessionKey: ref(''),
    autoScroll: ref(false),
    stream,
    normalizeElevatedMode: mode => mode,
    adoptResponseSession: vi.fn(),
    scheduleHistorySync: vi.fn(),
    schedulePendingDrainAfterTerminal: vi.fn(),
    flushDeferredPendingDrain: vi.fn(),
    isCompactInFlightForCurrentSession: () => false,
    hasPendingAttachmentWork: () => false,
    enqueuePendingInput: vi.fn(() => true),
    popAllPendingIntoComposer: vi.fn(() => false),
    executeSlashCommand: vi.fn(async () => false),
    closeSlashMenu: vi.fn(),
    autoResizeTextarea: vi.fn(),
    scrollToBottom: vi.fn(),
    ...overrides,
  }
  return { api: useChatSend(options), options, rpc, stream }
}

function sentParams(rpc: { call: ReturnType<typeof vi.fn> }, index = 0) {
  return rpc.call.mock.calls[index]?.[1] as Record<string, unknown>
}

describe('resolveSendWorkspaceIds', () => {
  it('主根固定在首位，去重保序', () => {
    expect(resolveSendWorkspaceIds('ws-a', ['ws-b', 'ws-a', 'ws-c', '']))
      .toEqual(['ws-a', 'ws-b', 'ws-c'])
  })

  it('只有主根时返回空数组（调用方据此完全不发 workspaceIds）', () => {
    expect(resolveSendWorkspaceIds('ws-a', [])).toEqual([])
    expect(resolveSendWorkspaceIds('ws-a', ['ws-a'])).toEqual([])
  })

  it('没有主根时不臆造多根绑定', () => {
    expect(resolveSendWorkspaceIds(null, ['ws-b', 'ws-c'])).toEqual([])
  })
})

describe('useChatSend 多根 workspaceIds', () => {
  it('主根 + 附加根时上行 workspaceIds，主根在首位', async () => {
    const pendingWorkspaceIds = ref(['ws-extra-1', 'ws-extra-2'])
    const boundWorkspaceId = ref<string | null>('ws-primary')
    const { api, rpc } = makeOptions({ pendingWorkspaceIds, boundWorkspaceId })

    await api.onSend()

    const params = sentParams(rpc)
    expect(params.workspaceId).toBe('ws-primary')
    expect(params.workspaceIds).toEqual(['ws-primary', 'ws-extra-1', 'ws-extra-2'])
  })

  it('只有单根时不发 workspaceIds（后端继续走旧的 workspaceId 路径）', async () => {
    const pendingWorkspaceIds = ref<string[]>([])
    const boundWorkspaceId = ref<string | null>('ws-primary')
    const { api, rpc } = makeOptions({ pendingWorkspaceIds, boundWorkspaceId })

    await api.onSend()

    const params = sentParams(rpc)
    expect(params.workspaceId).toBe('ws-primary')
    expect(Object.prototype.hasOwnProperty.call(params, 'workspaceIds')).toBe(false)
  })

  it('幂等回放比较包含 workspaceIds：附件不变但附加项目变了就是新请求', async () => {
    const pendingWorkspaceIds = ref(['ws-extra-1'])
    const boundWorkspaceId = ref<string | null>('ws-primary')
    const rpc = {
      call: vi.fn().mockRejectedValue(
        Object.assign(new Error('database busy'), { accepted: false }),
      ),
    }
    const { api } = makeOptions({ rpc, pendingWorkspaceIds, boundWorkspaceId })

    await api.onSend()
    const firstParams = sentParams(rpc)
    expect(firstParams.workspaceIds).toEqual(['ws-primary', 'ws-extra-1'])

    // 草稿与附件都没变 → 必须原样重放同一次请求（同 clientRequestId + 同参数）。
    await api.onSend()
    expect(sentParams(rpc, 1)).toEqual(firstParams)

    // 只改附加项目 → 旧指纹必须失效，否则「换了项目却复用旧请求」。
    pendingWorkspaceIds.value = ['ws-extra-2']
    await api.onSend()
    const thirdParams = sentParams(rpc, 2)
    expect(thirdParams.clientRequestId).not.toBe(firstParams.clientRequestId)
    expect(thirdParams.workspaceIds).toEqual(['ws-primary', 'ws-extra-2'])
  })
})
