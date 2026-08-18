import type { Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type {
  PendingQueuePayload,
  PendingQueueOwner,
} from '@/composables/chat/useChatPendingQueue'
import type { ToastTone } from '@/composables/useToasts'

/**
 * Composer 输入 / 发送控制器（C 阶段 — send/input 簇）。
 *
 * 迁入三个 composer 文本输入处理函数：
 * - sendComposerText: 把文本送入「待发送队列」或在输入框为空时直接触发发送；
 *   队列满 / 压缩进行中 / 发送被拦截时走对应分支。
 * - applyLandingSuggestion: 落地建议 chip 点击，经同一 composer 发送路径。
 * - appendComposerText: 语音输入 / steer 回填，把文本追加进输入框并聚焦。
 *
 * 诚实工程要点（高危缝保持原位）：
 * - `sendCurrentInput` 是迟绑定 `let` DI 缝（onSend / onComposerSend 在文件下方
 *   重赋值），不能随迁。改为经 `getSendCurrentInput` getter 在调用时读取当前绑定，
 *   缝仍留在 ChatView。
 * - `composerSendBlockedMessage` / `landingSuggestionsDisabled` 是晚于本控制器装配
 *   点（metaRuns 在 setup 期就把 sendComposerText 当参数传入）才声明的 computed。
 *   若直接传 ref 会在装配点触发 TDZ。故同样经 getter（`getComposerSendBlockedMessage`
 *   / `getLandingSuggestionsDisabled`）延后读取，引用保持闭包、运行时才求值。
 * - `pendingAttachments` / `isStreaming` 来自组合式返回值（非共享状态袋），以 ref 传入。
 * - `enqueuePendingPayload` / `schedulePendingDrainAfterTerminal` /
 *   `flushDeferredPendingDrain` 来自 chatPendingQueue；`pushToast` 来自 useToasts；
 *   `isCompactInFlightForCurrentSession` 是晚赋值 `let`，同样经 getter 读取。
 */
export interface ChatComposerInputControllerDeps {
  /** 延迟读取迟绑定 sendCurrentInput 缝（见文件头说明）。 */
  getSendCurrentInput: () => void
  enqueuePendingPayload: (payload: PendingQueuePayload, owner?: PendingQueueOwner) => boolean
  schedulePendingDrainAfterTerminal: () => void
  flushDeferredPendingDrain: () => void
  pushToast: (message: string, options?: { tone?: ToastTone; duration?: number }) => void
  /** 延迟读取晚赋值的 isCompactInFlightForCurrentSession `let`。 */
  getIsCompactInFlightForCurrentSession: () => boolean
  /** 延迟读取晚声明于本控制器装配点的 computed（避免 TDZ）。 */
  getComposerSendBlockedMessage: () => string
  /** 延迟读取晚声明于本控制器装配点的 computed（避免 TDZ）。 */
  getLandingSuggestionsDisabled: () => boolean
  pendingAttachments: Readonly<Ref<unknown[]>>
  isStreaming: Readonly<Ref<boolean>>
}

export function useChatComposerInputController(
  state: ChatViewState,
  deps: ChatComposerInputControllerDeps,
) {
  const { inputText, pendingQueueOwnerContext, sessionKey, composerRef } = state
  const { t } = useI18n()
  const {
    getSendCurrentInput,
    enqueuePendingPayload,
    schedulePendingDrainAfterTerminal,
    flushDeferredPendingDrain,
    pushToast,
    getIsCompactInFlightForCurrentSession,
    getComposerSendBlockedMessage,
    getLandingSuggestionsDisabled,
    pendingAttachments,
    isStreaming,
  } = deps

  // Queue-or-send dispatcher for composer text. An occupied composer (text or
  // attachments) stages the text as a pending queue item so an operator-owned
  // draft is never overwritten; an empty composer fires the live send path.
  function sendComposerText(text: string) {
    const next = String(text || '')
    if (!next) return

    if (inputText.value.trim() || pendingAttachments.value.length > 0) {
      const context = pendingQueueOwnerContext.value
      const owner = context?.sessionKey === sessionKey.value
        ? { ownerRequestId: context.ownerRequestId }
        : undefined
      const queued = enqueuePendingPayload({
        text: next,
        attachments: [],
        intent: null,
      }, owner)
      if (!queued) {
        pushToast(t('chat.toast.queueFull'), { tone: 'info' })
        return
      }
      if (!isStreaming.value && !getIsCompactInFlightForCurrentSession()) {
        schedulePendingDrainAfterTerminal()
        flushDeferredPendingDrain()
      }
      return
    }
    inputText.value = next
    composerRef.value?.resizeTextarea()
    if (getComposerSendBlockedMessage()) {
      composerRef.value?.focusTextarea()
      return
    }
    getSendCurrentInput()
  }

  // A suggestion chip is an explicit task choice. Route it through the same
  // composer-backed send path as every other message so routing, attachments,
  // optimistic state, and recovery behavior stay identical.
  function applyLandingSuggestion(text: string) {
    if (getLandingSuggestionsDisabled()) return
    sendComposerText(text)
  }

  // Voice transcription / steer replay append: extend the existing draft (never
  // replace it) and refocus the textarea so the operator keeps control.
  function appendComposerText(text: string) {
    const next = String(text || '').trim()
    if (!next) return
    inputText.value = inputText.value.trim()
      ? `${inputText.value.trimEnd()}\n${next}`
      : next
    composerRef.value?.resizeTextarea()
    composerRef.value?.focusTextarea()
  }

  return {
    sendComposerText,
    applyLandingSuggestion,
    appendComposerText,
  }
}
