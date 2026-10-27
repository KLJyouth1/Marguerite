import { describe, expect, it } from 'vitest'
import { shouldShowConfirmedEmptySession } from '@/utils/chat/sessionLoadState'
import chatViewSource from './ChatView.vue?raw'

// 真机缺陷（2026-10-24，安装版 0.2.16）：`/chat` 首次进入能看到「暂无消息。」，
// 刷新后聊天区完全空白。刷新时 lastRoute 只恢复路径 `/chat`、丢掉 `?session=`，
// 视图因此重建一个临时草稿：草稿不请求历史，`initialHistoryStatus` 永远停在
// `pending`，而原来的空态判定只认 `ready`；同时 `/chat` 不是草稿路由，
// 新建会话落地页也不渲染 —— 两条分支都落空，聊天区就什么都没有。
describe('ChatView — 空会话空状态', () => {
  it('刷新后重建的空白草稿也渲染空状态（无消息 + 不请求历史）', () => {
    expect(shouldShowConfirmedEmptySession({
      isDraftLanding: false,
      isStreaming: false,
      messageCount: 0,
      initialHistoryStatus: 'pending',
      isNewSession: true,
    })).toBe(true)
  })

  it('历史已水合（ready）的空白会话渲染空状态', () => {
    expect(shouldShowConfirmedEmptySession({
      isDraftLanding: false,
      isStreaming: false,
      messageCount: 0,
      initialHistoryStatus: 'ready',
      isNewSession: false,
    })).toBe(true)
  })

  it('历史仍在读取/失败时不误报空会话', () => {
    for (const initialHistoryStatus of ['pending', 'loading', 'error'] as const) {
      expect(shouldShowConfirmedEmptySession({
        isDraftLanding: false,
        isStreaming: false,
        messageCount: 0,
        initialHistoryStatus,
        isNewSession: false,
      })).toBe(false)
    }
  })

  it('草稿落地页用引导卡代替空状态文案', () => {
    expect(shouldShowConfirmedEmptySession({
      isDraftLanding: true,
      isStreaming: false,
      messageCount: 0,
      initialHistoryStatus: 'pending',
      isNewSession: true,
    })).toBe(false)
  })

  it('ChatView 模板把空状态绑定到该判定并渲染「暂无消息」文案', () => {
    expect(chatViewSource).toContain('v-if="!forkTransition && showConfirmedEmptySession"')
    expect(chatViewSource).toContain('class="chat-empty"')
    expect(chatViewSource).toContain("t('chat.noMessagesYet')")
    expect(chatViewSource).toContain("isNewSession: pendingSessionIntent.value === 'new_chat'")
  })
})
