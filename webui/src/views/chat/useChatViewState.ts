// 共享状态层（B 阶段基础模块）。
//
// ChatView.vue 的装配层原本在单一 setup 作用域内声明约 40 个响应式 ref，
// 彼此强耦合，导致无法安全拆分。本 composable 作为这些共享状态的「单一归属」，
// 本回合一次性完成全部本地 ref 的迁移（B 阶段状态层收口），后续控制器拆分
// （useChatSend / useChatStream / useChatSession / useChatHistory / useChatShare 等）
// 将在此状态层之上按需抽取，每刀仍以 vitest 门禁。
import { ref, type Ref } from 'vue'
import type { ToolResultContext, ChatRunStatus } from '@/types/chat'
import type { ChatComposerHandle, Message, ChatHeaderActionsHandle } from './chat-view.types'
import type { ShareExportTheme } from '@/composables/chat/useChatShareExport'
import type { InterruptViewState } from '@/types/parts'
import type { SandboxRunMode } from '@/types/sandbox'
import type { ForkTransitionState } from '@/utils/chat/forkTransition'
import type { PromptCacheKeepaliveStatus } from '@/types/promptCacheKeepalive'
import type { PlanRunSnapshot } from '@/types/plans'
import type { PendingQueueOwnerContext } from '@/composables/chat/useChatPendingQueue'

/** 工具结果弹窗的开关与内容状态。 */
export interface ToolResultModalState {
  open: boolean
  title: string
  content: string
  context?: ToolResultContext
}

/**
 * ChatView 共享状态聚合对象。
 * 本回合完成「全部本地 ref」迁移，作为后续控制器拆分的基础状态袋（state bag）。
 */
export interface ChatViewState {
  /** 工具结果弹窗状态（首个迁入的共享状态单元）。 */
  toolResultModal: Ref<ToolResultModalState>
  /** 输入框文本（composer 输入状态；被 send/draft/sendComposerText 等约 30 处共享读写）。 */
  inputText: Ref<string>
  /** composer 是否处于 IME 组字中（由 ChatComposer 的 @composition-change 上报；纯本地状态）。 */
  composing: Ref<boolean>
  /** 请求是否已中止（由发送/中断逻辑置位，按引用透传给多个 composable；纯本地状态）。 */
  aborted: Ref<boolean>

  // —— 以下为一次性迁入的剩余共享 ref ——
  projectPickerOpen: Ref<boolean>
  landingPrefilled: Ref<boolean>
  pendingAutoSend: Ref<string>
  pendingAutoSendSessionKey: Ref<string>
  threadRef: Ref<HTMLElement | null>
  composerRef: Ref<ChatComposerHandle | null>
  composerCollapsed: Ref<boolean>
  chatHeaderActionsRef: Ref<ChatHeaderActionsHandle | null>
  sessionKey: Ref<string>
  promptCacheKeepaliveOpen: Ref<boolean>
  promptCacheKeepaliveStatus: Ref<PromptCacheKeepaliveStatus | null>
  composerRevision: Ref<number>
  autoScroll: Ref<boolean>
  messages: Ref<Message[]>
  forkTransition: Ref<ForkTransitionState | null>
  lastHeaderRole: Ref<string>
  lastHeaderDay: Ref<string>
  threadDragOver: Ref<boolean>
  threadDragDepth: Ref<number>
  shareMode: Ref<boolean>
  shareSaving: Ref<boolean>
  selectedShareMessageIds: Ref<Set<string>>
  shareBannerRef: Ref<HTMLElement | null>
  sharePreview: Ref<{ url: string; blob: Blob; filename: string } | null>
  shareTheme: Ref<ShareExportTheme>
  activeRunModeLock: Ref<SandboxRunMode | null>
  composerSandboxSetupOpen: Ref<boolean>
  runStatus: Ref<ChatRunStatus>
  currentEpoch: Ref<number>
  lastStreamSeq: Ref<number>
  activeTaskGroups: Ref<Set<string>>
  activeStreamTaskId: Ref<string>
  activeStreamSessionKey: Ref<string>
  pendingSessionIntent: Ref<string | null>
  pendingForkBeforeMessageId: Ref<string | null>
  interruptState: Ref<ReadonlyMap<string, InterruptViewState>>
  pendingQueueOwnerContext: Ref<PendingQueueOwnerContext | null>
  compactInFlight: Ref<boolean>
  metaSetupProviderNavigationPending: Ref<boolean>
  turnSettledAnnouncement: Ref<string>
  executionDockRun: Ref<PlanRunSnapshot | null>
  deliverablesOpen: Ref<boolean>
}

export function useChatViewState(
  t: (key: string) => string = (key) => key,
): ChatViewState {
  const toolResultModal = ref<ToolResultModalState>({
    open: false,
    title: '',
    content: '',
    context: undefined,
  })

  // composer 输入框文本：单一归属，避免 ChatView 装配层重复声明并被多方耦合。
  const inputText = ref('')
  // IME 组字态：仅被 ChatComposer 的 @composition-change 写入、useChatComposerShortcuts 按引用接收。
  const composing = ref(false)
  // 中止态：由发送/中断逻辑置位，按引用透传给 useChatSend / useChatStream 等 composable。
  const aborted = ref(false)

  // —— 一次性迁入的剩余共享 ref（绑定名保持与 ChatView 内引用一致）——
  const projectPickerOpen = ref(false)
  const landingPrefilled = ref(false)
  const pendingAutoSend = ref('')
  const pendingAutoSendSessionKey = ref('')
  const threadRef = ref<HTMLElement | null>(null)
  const composerRef = ref<ChatComposerHandle | null>(null)
  const composerCollapsed = ref(false)
  const chatHeaderActionsRef = ref<ChatHeaderActionsHandle | null>(null)
  const sessionKey = ref('')
  const promptCacheKeepaliveOpen = ref(false)
  const promptCacheKeepaliveStatus = ref<PromptCacheKeepaliveStatus | null>(null)
  const composerRevision = ref(0)
  const autoScroll = ref(true)
  const messages = ref<Message[]>([])
  const forkTransition = ref<ForkTransitionState | null>(null)
  const lastHeaderRole = ref('')
  const lastHeaderDay = ref('')
  const threadDragOver = ref(false)
  const threadDragDepth = ref(0)
  const shareMode = ref(false)
  const shareSaving = ref(false)
  const selectedShareMessageIds = ref<Set<string>>(new Set())
  const shareBannerRef = ref<HTMLElement | null>(null)
  const sharePreview = ref<{ url: string; blob: Blob; filename: string } | null>(null)
  const shareTheme = ref<ShareExportTheme>('light')
  const activeRunModeLock = ref<SandboxRunMode | null>(null)
  const composerSandboxSetupOpen = ref(false)
  const runStatus = ref<ChatRunStatus>({
    status: 'idle',
    label: t('chat.status.idle'),
    task: null,
  })
  const currentEpoch = ref(0)
  const lastStreamSeq = ref(0)
  const activeTaskGroups = ref<Set<string>>(new Set())
  const activeStreamTaskId = ref<string>('')
  const activeStreamSessionKey = ref<string>('')
  const pendingSessionIntent = ref<string | null>(null)
  const pendingForkBeforeMessageId = ref<string | null>(null)
  const interruptState = ref<ReadonlyMap<string, InterruptViewState>>(new Map())
  const pendingQueueOwnerContext = ref<PendingQueueOwnerContext | null>(null)
  const compactInFlight = ref(false)
  const metaSetupProviderNavigationPending = ref(false)
  const turnSettledAnnouncement = ref('')
  const executionDockRun = ref<PlanRunSnapshot | null>(null)
  const deliverablesOpen = ref(false)

  return {
    toolResultModal,
    inputText,
    composing,
    aborted,
    projectPickerOpen,
    landingPrefilled,
    pendingAutoSend,
    pendingAutoSendSessionKey,
    threadRef,
    composerRef,
    composerCollapsed,
    chatHeaderActionsRef,
    sessionKey,
    promptCacheKeepaliveOpen,
    promptCacheKeepaliveStatus,
    composerRevision,
    autoScroll,
    messages,
    forkTransition,
    lastHeaderRole,
    lastHeaderDay,
    threadDragOver,
    threadDragDepth,
    shareMode,
    shareSaving,
    selectedShareMessageIds,
    shareBannerRef,
    sharePreview,
    shareTheme,
    activeRunModeLock,
    composerSandboxSetupOpen,
    runStatus,
    currentEpoch,
    lastStreamSeq,
    activeTaskGroups,
    activeStreamTaskId,
    activeStreamSessionKey,
    pendingSessionIntent,
    pendingForkBeforeMessageId,
    interruptState,
    pendingQueueOwnerContext,
    compactInFlight,
    metaSetupProviderNavigationPending,
    turnSettledAnnouncement,
    executionDockRun,
    deliverablesOpen,
  }
}
