import { readFileSync } from 'node:fs'

import { describe, expect, it } from 'vitest'

import messageListSource from '../components/chat/ChatMessageList.vue?raw'
import historySource from '../composables/chat/useChatHistory.ts?raw'
import transitionSource from '../utils/chat/forkTransition.ts?raw'
import chatViewSource from './ChatView.vue?raw'
import forkControllerSource from '../composables/chat/useChatForkController.ts?raw'
import forkOverlaySource from '../components/chat/ChatForkTransitionOverlay.vue?raw'

const chatViewStyles = readFileSync(
  new URL('../styles/chat-view.css', import.meta.url),
  'utf8',
)

// Fork orchestration moved out of ChatView.vue into useChatForkController.
// Assertions are split by owner: template/mount-order/CSS stay on the component
// and its overlay, the hand-off state machine is asserted on the controller, and
// a wiring group pins the seam between them so neither side can drift into dead
// code.
describe('chat fork hand-off contract', () => {
  it('wires ChatView to the fork controller', () => {
    expect(chatViewSource).toContain('useChatForkController(viewState, {')
    expect(chatViewSource).toContain('forkConversation,')
    expect(chatViewSource).toContain('applyForkRouteHandoff(newSession)')
    expect(chatViewSource).toContain('syncForkFromHistoryStatus(activeKey, loadedKey, status)')
    expect(chatViewSource).toContain('syncForkFromLivePhase(activeKey, phase)')
    // The liveness guard reads mutable `let` primitives, so it must be injected
    // as getters — passing the booleans by value would freeze a stale snapshot.
    expect(chatViewSource).toContain('isViewActive: () => chatViewActive,')
    expect(chatViewSource).toContain('isViewDisposed: () => chatViewDisposed,')
  })

  it('uses durable inclusive turn ids for historical assistant branches', () => {
    expect(messageListSource).toContain("forkConversation: [throughTurnId?: string]")
    expect(messageListSource).toContain("turnOutcome?.turnId?.trim()")
    expect(messageListSource).toContain("@fork=\"$emit('forkConversation', forkThroughTurnId(index))\"")
    expect(transitionSource).toContain("method: 'sessions.forkThroughTurn'")
    expect(transitionSource).toContain("params: { key: parentKey, throughTurnId }")
    expect(forkControllerSource).toContain('rpc.call<ForkRpcResponse>(request.method, request.params)')
    expect(forkControllerSource).toContain('validatedForkChildKey(res, normalizedTurnId)')
    expect(forkControllerSource.indexOf('validatedForkChildKey(res, normalizedTurnId)')).toBeLessThan(
      forkControllerSource.indexOf('query: { session: childKey }'),
    )
    expect(transitionSource).toContain("response?.forkMode !== 'through_turn'")
    expect(transitionSource).toContain('response.throughTurnId !== throughTurnId')
    expect(forkControllerSource).not.toContain('beforeMessageId: normalizedTurnId')
    expect(chatViewSource).not.toContain('beforeMessageId: normalizedTurnId')
    expect(forkControllerSource).toContain(
      "clearForkTransition(generation)\n        pushToast(t('chat.toast.forkFailed')",
    )
  })

  it('keeps a read-only parent projection visible until child history is ready', () => {
    expect(chatViewSource).toContain(':messages="forkTransition?.previewMessages || renderedMessages"')
    expect(chatViewSource).toContain(':session-key="forkTransition?.parentKey || sessionKey"')
    expect(chatViewSource).toContain(':inert="forkTransition ? true : undefined"')
    expect(forkControllerSource).toContain('Render-only snapshot; never becomes the child session\'s canonical messages.')
    expect(forkControllerSource).toContain('loadedKey !== transition.targetKey')
    expect(forkControllerSource).toContain("if (status === 'ready')")
    expect(historySource).toContain('historySessionKey.value = key')
    expect(historySource).toContain('historySessionKey,')
    expect(transitionSource).toContain('matchedBoundary = index')
    expect(transitionSource).toContain('source.slice(0, boundary + 1)')
  })

  it('renders status as an absolute shell sibling without shifting messages', () => {
    expect(forkOverlaySource).toContain('class="chat-fork-transition-overlay"')
    // 子组件在 ChatView 中的挂载点仍位于 thread 之前（绝对定位兄弟，不挤压消息）
    expect(chatViewSource.indexOf('ChatForkTransitionOverlay')).toBeLessThan(
      chatViewSource.indexOf('ref="threadRef"'),
    )
    expect(chatViewStyles).toMatch(
      /\.chat-fork-transition-overlay\s*\{[^}]*position:\s*absolute[^}]*inset-inline:\s*0[^}]*pointer-events:\s*none/s,
    )
    expect(chatViewStyles).toMatch(
      /\.chat-fork-transition-status\s*\{[^}]*max-width:[^}]*pointer-events:\s*auto/s,
    )
    expect(chatViewStyles).toMatch(
      /\.chat-fork-transition-status--error\s*\{[^}]*flex-wrap:\s*wrap/s,
    )
    expect(chatViewStyles).toMatch(
      /\.chat-fork-transition-status__action\s*\{[^}]*max-width:\s*100%[^}]*white-space:\s*normal/s,
    )
  })

  it('keeps the preview while returning until parent history is ready', () => {
    expect(transitionSource).toContain("export type ForkTransitionPhase = 'creating' | 'opening' | 'returning' | 'error'")
    expect(forkControllerSource).toContain('type ForkTransitionState,')
    expect(forkControllerSource).toContain('targetKey: transition.parentKey')
    expect(forkControllerSource).toContain("phase: 'returning'")
    expect(forkControllerSource).toContain('activeKey !== transition.targetKey')
    expect(forkControllerSource).toContain('loadedKey !== transition.targetKey')
    expect(transitionSource).toContain("targetKey === parentKey ? 'returning' : 'opening'")
    expect(forkControllerSource).toContain('forkRouteHandoffAction(newSession, transition)')
    expect(forkControllerSource).toContain("if (handoffAction === 'returning')")
  })

  it('invalidates every asynchronous fork continuation when the view unmounts', () => {
    // Disposal stays with the component: it owns the lifecycle hook.
    expect(chatViewSource).toContain('forkTransitionLifetime.dispose()')
    expect(chatViewSource).toContain('forkTransition.value = null')
    // The guard itself now lives in the controller and reads the injected getters.
    expect(forkControllerSource).toContain('isViewActive()\n      && !isViewDisposed()')
    expect(forkControllerSource).toContain('if (!isForkTransitionActive(generation)) return')
    expect(transitionSource).toContain('export function createForkTransitionLifetime()')
  })

  it('keeps busy through navigation and retains recovery controls on child failures', () => {
    const failureHandler = forkControllerSource.slice(
      forkControllerSource.indexOf('function failForkTransition('),
      forkControllerSource.indexOf('async function retryForkTransition()'),
    )
    // `forkInFlight` still derives busy state in the component.
    expect(chatViewSource).toContain("forkTransition.value.phase !== 'error'")
    expect(forkControllerSource).toContain('const navigationFailure = await router.push')
    expect(forkControllerSource).not.toContain("router.push({ path: '/chat', query: { session: childKey } }).catch")
    expect(forkControllerSource).toContain("transition.generation,\n        'history',")
    expect(forkControllerSource).toContain("transition.generation,\n        'live',")
    expect(forkControllerSource).toContain("phase: 'error'")
    expect(failureHandler).toContain("phase: 'error'")
    expect(failureHandler).not.toContain('clearForkTransition(')
    expect(forkOverlaySource).toContain('data-testid="chat-fork-retry"')
    expect(forkOverlaySource).toContain('data-testid="chat-fork-return"')
    expect(forkControllerSource).toContain("pushToast(t('chat.toast.forkOpenFailed'), { tone: 'warn' })")
  })
})
