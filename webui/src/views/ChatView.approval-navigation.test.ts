import { describe, expect, it } from 'vitest'
import appSource from '@/App.vue?raw'
import chatViewSource from './ChatView.vue?raw'
import composerControllerSource from '@/composables/chat/useChatComposerController.ts?raw'

describe('pending approval navigation wiring', () => {
  it('turns the topbar approval button into a session route plus exact-card focus request', () => {
    expect(appSource).toContain('appStore.requestApprovalFocus(oldest)')
    expect(appSource).toContain(
      `switchToSession(oldest.sessionKey, 'approval.openBlockedSession')`,
    )
  })

  it('clears stale global approval attention while the gateway is disconnected', () => {
    expect(appSource).toContain("if (state !== 'connected')")
    expect(appSource).toContain('appStore.setPendingApprovals([])')
  })

  it('keeps new pending approvals at the bottom and handles exact focus requests', () => {
    expect(chatViewSource).toContain('focusPendingApprovalCard')
    expect(chatViewSource).toContain('livePendingInterruptParts')
    expect(chatViewSource).toMatch(/autoScroll\.value\s*=\s*true/)
    expect(chatViewSource).toContain('scrollToBottom()')
    // Batch 8: 两个辅助函数体已迁入 useChatComposerController，ChatView 仅保留装配接线。
    expect(composerControllerSource).toContain('async function focusPendingApprovalCard')
    expect(composerControllerSource).toContain('function scrollToBottom')
    expect(chatViewSource).toContain('useChatComposerController(viewState')
  })
})
