import { nextTick } from 'vue'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import { cssEscapeAttr } from '@/composables/chat/chatViewUtils'
import { useAppStore } from '@/stores/app'

/**
 * Composer 辅助控制器（C 阶段 Batch 8 — 安全自包含切片）。
 *
 * 迁入四个零跨函数耦合的纯滚动/审批辅助函数：
 * - scrollToBottom: 流式跟随到底部（受 autoScroll 闸门保护）
 * - lastUserMessageText: 最近一条用户消息文本（供 MetaRuns 回填草稿）
 * - scrollToStepCard: 线程内工具卡滚动定位（meta step chip 点击 / 详情）
 * - focusPendingApprovalCard: 精准聚焦待审批卡片（审批导航）
 *
 * 这些函数只读取共享状态袋（threadRef / autoScroll / messages / sessionKey）
 * 与全局 appStore，不写入任何编排侧本地状态，故可安全迁出。
 *
 * 高危编排（onSend / onComposerSend / sendCurrentInput 迟绑定 DI 缝、fork 跨
 * watcher/生命周期、thread scroll & retraction 6 函数、showJumpToLatest /
 * jumpToLatest、plans dock 6、autoResizeTextarea）按诚实工程原则留 ChatView，
 * 不在本批盲拆。
 */
export function useChatComposerController(state: ChatViewState) {
  const { threadRef, autoScroll, messages, sessionKey } = state
  const appStore = useAppStore()

  function scrollToBottom() {
    nextTick(() => {
      // A stream/event may request a follow while the reader is at the live edge,
      // then the reader can scroll up before Vue applies this next-tick callback.
      // Re-check here so that queued automatic scrolls never override that choice.
      if (threadRef.value && autoScroll.value) {
        threadRef.value.scrollTop = threadRef.value.scrollHeight
      }
    })
  }

  // The most recent user message text (mirrors vanilla `_latestUserMessageText`).
  function lastUserMessageText(): string {
    for (let i = messages.value.length - 1; i >= 0; i--) {
      if (messages.value[i]?.role === 'user') return messages.value[i].text || ''
    }
    return ''
  }

  // Resolve a step's in-thread tool card and scroll it into view (chip click /
  // show-detail). The card carries data-tool-use-id="meta_step_<id>".
  function scrollToStepCard(toolUseId: string) {
    const root = threadRef.value
    if (!root) return
    const card = root.querySelector(`[data-tool-use-id="${cssEscapeAttr(toolUseId)}"]`)
    if (card && typeof (card as HTMLElement).scrollIntoView === 'function') {
      const reduceMotion = typeof window !== 'undefined'
        && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
      ;(card as HTMLElement).scrollIntoView({ block: 'center', behavior: reduceMotion ? 'auto' : 'smooth' })
    }
  }

  async function focusPendingApprovalCard() {
    const request = appStore.approvalFocusRequest
    if (!request || request.sessionKey !== sessionKey.value) return

    await nextTick()
    if (
      appStore.approvalFocusRequest?.requestId !== request.requestId
      || request.sessionKey !== sessionKey.value
    ) return

    const card = [...(threadRef.value?.querySelectorAll<HTMLElement>('[data-approval-id]') ?? [])]
      .find(element => element.dataset.approvalId === request.approvalId)
    if (!card) return

    card.scrollIntoView({ behavior: 'smooth', block: 'center' })
    card.focus({ preventScroll: true })
    appStore.clearApprovalFocusRequest(request.requestId)
  }

  return {
    scrollToBottom,
    lastUserMessageText,
    scrollToStepCard,
    focusPendingApprovalCard,
  }
}
