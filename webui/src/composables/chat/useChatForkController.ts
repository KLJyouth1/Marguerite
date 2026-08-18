// 会话分叉（fork）控制器（从 ChatView.vue 抽出的本地编排函数）。
//
// 分叉编排在 ChatView 里原本散落在四处：7 个本地函数、路由 watcher 的内联
// hand-off 分支、history 状态 watcher、live 相位 watcher。这里把它们全部收成
// 一组显式导出——watcher 仍由 ChatView 注册（它拥有生命周期），但 watcher 体
// 只调用本控制器的 syncForkFrom* / applyForkRouteHandoff，不再自己改 fork 状态。
//
// 关键约束：`chatViewActive` / `chatViewDisposed` 在 ChatView 里是可变的 `let`
// 原语，按值传入会被快照冻结。因此以 getter（isViewActive / isViewDisposed）
// 注入，读取时机才求值。
import type { Ref, ComputedRef } from 'vue'
import type { Router } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type { ChatRenderedMessage } from '@/types/chat'
import type { ChatHistoryState } from '@/composables/chat/useChatHistory'
import type { SessionLivePhase } from '@/composables/chat/sessionBootstrapContract'
import {
  createForkTransitionLifetime,
  forkNavigationPhase,
  forkRouteHandoffAction,
  forkRpcRequest,
  snapshotForkPreviewMessages,
  validatedForkChildKey,
  type ForkRpcResponse,
  type ForkTransitionState,
} from '@/utils/chat/forkTransition'

export interface ChatForkControllerDeps {
  rpc: ReturnType<typeof useRpcStore>
  router: Router
  /** ChatView 拥有的分叉世代守卫，跨异步续跑用它判定新鲜度。 */
  forkTransitionLifetime: ReturnType<typeof createForkTransitionLifetime>
  /** 读取 ChatView 的 `chatViewActive`（可变 let，必须惰性取值）。 */
  isViewActive: () => boolean
  /** 读取 ChatView 的 `chatViewDisposed`（可变 let，必须惰性取值）。 */
  isViewDisposed: () => boolean
  /** 从 URL 读取当前 session，用于识别 router.push 的静默失败。 */
  readSessionFromUrl: () => string | null
  renderedMessages: Readonly<ComputedRef<ChatRenderedMessage[]>>
  isStreaming: Readonly<Ref<boolean>> | Readonly<ComputedRef<boolean>>
  livePhase: Readonly<Ref<SessionLivePhase>>
  historySessionKey: Readonly<Ref<string>>
  historyState: Readonly<Ref<ChatHistoryState>>
  retryHistory: () => void
  retryLive: () => void
}

export function useChatForkController(
  state: ChatViewState,
  deps: ChatForkControllerDeps,
) {
  const { t } = useI18n()
  const { pushToast } = useToasts()
  const { forkTransition, sessionKey, pendingSessionIntent, messages } = state
  const {
    rpc,
    router,
    forkTransitionLifetime,
    isViewActive,
    isViewDisposed,
    readSessionFromUrl,
    renderedMessages,
    isStreaming,
    livePhase,
    historySessionKey,
    historyState,
    retryHistory,
    retryLive,
  } = deps

  function clearForkTransition(generation?: number) {
    if (
      generation !== undefined
      && forkTransition.value?.generation !== generation
    ) return
    const clearedGeneration = generation ?? forkTransition.value?.generation
    forkTransition.value = null
    forkTransitionLifetime.invalidate(clearedGeneration)
  }

  function isForkTransitionActive(generation: number): boolean {
    return Boolean(
      isViewActive()
      && !isViewDisposed()
      && forkTransitionLifetime.isCurrent(generation)
      && forkTransition.value?.generation === generation
    )
  }

  function failForkTransition(
    generation: number,
    reason: NonNullable<ForkTransitionState['errorReason']>,
    error: unknown,
  ) {
    if (!isForkTransitionActive(generation)) return
    const transition = forkTransition.value!
    console.warn('Fork child hand-off failed:', error instanceof Error ? error.message : error)
    const firstFailure = transition.phase !== 'error'
    forkTransition.value = {
      ...transition,
      phase: 'error',
      errorReason: reason,
    }
    if (firstFailure) pushToast(t('chat.toast.forkOpenFailed'), { tone: 'warn' })
  }

  async function retryForkTransition() {
    const transition = forkTransition.value
    if (
      !transition?.targetKey
      || transition.phase !== 'error'
      || !isForkTransitionActive(transition.generation)
    ) return
    const retryPhase = forkNavigationPhase(transition.targetKey, transition.parentKey)
    forkTransition.value = {
      ...transition,
      phase: retryPhase,
      errorReason: undefined,
    }
    try {
      if (
        sessionKey.value !== transition.targetKey
        || readSessionFromUrl() !== transition.targetKey
      ) {
        const navigationFailure = await router.push({
          path: '/chat',
          query: { session: transition.targetKey },
        })
        if (!isForkTransitionActive(transition.generation)) return
        if (navigationFailure && readSessionFromUrl() !== transition.targetKey) {
          throw navigationFailure
        }
        return
      }
      if (livePhase.value === 'degraded') void retryLive()
      void retryHistory()
    } catch (error) {
      failForkTransition(transition.generation, 'navigation', error)
    }
  }

  async function returnToForkParent() {
    const transition = forkTransition.value
    if (!transition || !isForkTransitionActive(transition.generation)) return
    if (
      sessionKey.value === transition.parentKey
      && readSessionFromUrl() === transition.parentKey
    ) {
      if (
        transition.targetKey === transition.parentKey
        && historySessionKey.value === transition.parentKey
        && historyState.value.initialLoadStatus !== 'ready'
      ) {
        if (transition.phase === 'error') {
          await retryForkTransition()
          if (!isForkTransitionActive(transition.generation)) return
        }
        return
      }
      clearForkTransition(transition.generation)
      return
    }
    forkTransition.value = {
      ...transition,
      targetKey: transition.parentKey,
      phase: 'returning',
      errorReason: undefined,
    }
    try {
      const navigationFailure = await router.push({
        path: '/chat',
        query: { session: transition.parentKey },
      })
      if (!isForkTransitionActive(transition.generation)) return
      if (navigationFailure && readSessionFromUrl() !== transition.parentKey) {
        throw navigationFailure
      }
    } catch (error) {
      failForkTransition(transition.generation, 'navigation', error)
    }
  }

  async function forkConversation(throughTurnId?: string) {
    const parentKey = sessionKey.value
    if (!parentKey || forkTransition.value) return
    if (pendingSessionIntent.value === 'new_chat' || isStreaming.value) return
    const normalizedTurnId = throughTurnId?.trim() || undefined
    const generation = forkTransitionLifetime.begin()
    if (!generation) return
    forkTransition.value = {
      generation,
      parentKey,
      childKey: '',
      targetKey: parentKey,
      ...(normalizedTurnId ? { throughTurnId: normalizedTurnId } : {}),
      phase: 'creating',
      // Render-only snapshot; never becomes the child session's canonical messages.
      previewMessages: snapshotForkPreviewMessages(renderedMessages.value, normalizedTurnId),
    }
    try {
      const request = forkRpcRequest(parentKey, normalizedTurnId)
      const res = await rpc.call<ForkRpcResponse>(request.method, request.params)
      if (!isForkTransitionActive(generation)) return
      const childKey = validatedForkChildKey(res, normalizedTurnId)
      if (sessionKey.value !== parentKey) {
        clearForkTransition(generation)
        return
      }
      forkTransition.value = {
        ...forkTransition.value,
        childKey,
        targetKey: childKey,
        phase: 'opening',
      }
      const navigationFailure = await router.push({
        path: '/chat',
        query: { session: childKey },
      })
      if (!isForkTransitionActive(generation)) return
      if (navigationFailure && readSessionFromUrl() !== childKey) {
        throw navigationFailure
      }
    } catch (err) {
      if (!isForkTransitionActive(generation)) return
      const childCreated = Boolean(forkTransition.value?.childKey)
      if (childCreated) {
        failForkTransition(generation, 'navigation', err)
      } else if (forkTransition.value?.generation === generation) {
        console.warn('Fork failed:', err)
        clearForkTransition(generation)
        pushToast(t('chat.toast.forkFailed'), { tone: 'danger' })
      }
    }
  }

  // Owner recovery for a run paused by the sandbox denial ledger (the terminal
  // error card exposes a Resume button). Clearing the pause lets the next turn
  // proceed; the run itself already ended, so we prompt the user to resend.
  async function resumeSandbox() {
    const key = sessionKey.value
    if (!key) return
    try {
      await rpc.call('sandbox.resume', { sessionKey: key })
      messages.value.push({
        role: 'system',
        text: t('chat.sandboxResumed'),
        ts: new Date().toISOString(),
      })
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err)
      pushToast(t('chat.sandboxResumeFailed', { error: detail }), { tone: 'danger' })
    }
  }

  /**
   * Route-change hand-off. A navigation away from the child either means the
   * user is heading back to the parent (keep the preview, flip to `returning`)
   * or left the fork entirely (drop the transition).
   */
  function applyForkRouteHandoff(newSession: unknown) {
    const transition = forkTransition.value
    if (!transition) return
    const handoffAction = forkRouteHandoffAction(newSession, transition)
    if (handoffAction === 'returning') {
      forkTransition.value = {
        ...transition,
        targetKey: transition.parentKey,
        phase: 'returning',
        errorReason: undefined,
      }
    } else if (handoffAction === 'clear') {
      clearForkTransition(transition.generation)
    }
  }

  /**
   * A route switch briefly retains the parent's terminal history status. Require
   * the history composable to bind to the child key before treating `ready` as
   * hand-off completion, otherwise the preview would disappear one tick early.
   */
  function syncForkFromHistoryStatus(
    activeKey: string,
    loadedKey: string,
    status: ChatHistoryState['initialLoadStatus'],
  ) {
    const transition = forkTransition.value
    if (
      !transition
      || transition.phase === 'creating'
      || !transition.targetKey
      || activeKey !== transition.targetKey
      || loadedKey !== transition.targetKey
    ) return
    if (status === 'ready') {
      clearForkTransition(transition.generation)
    } else if (status === 'error') {
      failForkTransition(
        transition.generation,
        'history',
        new Error('Child history failed to load'),
      )
    }
  }

  /** A degraded live subscription on the fork target is a hand-off failure. */
  function syncForkFromLivePhase(activeKey: string, phase: SessionLivePhase) {
    const transition = forkTransition.value
    if (
      transition?.phase !== 'creating'
      && transition?.targetKey === activeKey
      && phase === 'degraded'
    ) {
      failForkTransition(
        transition.generation,
        'live',
        new Error('Child live subscription unavailable'),
      )
    }
  }

  return {
    clearForkTransition,
    isForkTransitionActive,
    failForkTransition,
    retryForkTransition,
    returnToForkParent,
    forkConversation,
    resumeSandbox,
    applyForkRouteHandoff,
    syncForkFromHistoryStatus,
    syncForkFromLivePhase,
  }
}
