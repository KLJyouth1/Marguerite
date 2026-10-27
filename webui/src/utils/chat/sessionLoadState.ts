export type InitialHistoryLoadStatus = 'pending' | 'loading' | 'ready' | 'error'
export type ChatHistoryRecoveryState =
  | 'history-loading'
  | 'history-retrying'
  | 'history-error'
  | null
export type ChatSessionRecoveryState =
  | Exclude<ChatHistoryRecoveryState, null>
  | 'live-connecting'
  | 'live-degraded'

export interface ResolveChatHistoryRecoveryStateOptions {
  isDraftLanding: boolean
  initialHistoryStatus: InitialHistoryLoadStatus
  retrying: boolean
  recoveryError?: boolean
}

export function resolveChatHistoryRecoveryState(
  options: ResolveChatHistoryRecoveryStateOptions,
): ChatHistoryRecoveryState {
  if (options.isDraftLanding) return null
  if (options.retrying) return 'history-retrying'
  if (options.recoveryError || options.initialHistoryStatus === 'error') {
    return 'history-error'
  }
  if (
    options.initialHistoryStatus === 'pending'
    || options.initialHistoryStatus === 'loading'
  ) return 'history-loading'
  return null
}

export function visibleChatHistoryRecoveryState(
  state: ChatHistoryRecoveryState,
): ChatHistoryRecoveryState {
  return state === 'history-loading' ? null : state
}

export function shouldShowConfirmedEmptySession(options: {
  isDraftLanding: boolean
  isStreaming: boolean
  messageCount: number
  initialHistoryStatus: InitialHistoryLoadStatus
  /**
   * 新建草稿会话（`/chat` 直接进入或刷新后重建）：它从不请求历史，
   * `initialHistoryStatus` 会永远停在 `pending`。此时「没有历史可读」本身就是
   * 权威空态，不能继续等一个永远不会到达的 `ready`，否则整块聊天区全空白。
   */
  isNewSession?: boolean
}): boolean {
  if (options.isDraftLanding || options.isStreaming || options.messageCount !== 0) {
    return false
  }
  if (options.initialHistoryStatus === 'ready') return true
  return options.isNewSession === true && options.initialHistoryStatus === 'pending'
}
