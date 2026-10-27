import { describe, expect, it } from 'vitest'
import { resolveSendWorkspaceId } from './useChatSend'

const base = {
  hasWorkspaceField: false,
  payloadWorkspaceId: null as string | null,
  intent: 'followup' as string | null,
  pendingWorkspaceId: null as string | null,
  boundWorkspaceId: null as string | null,
}

describe('resolveSendWorkspaceId', () => {
  it('新任务用草稿的 pending 绑定', () => {
    expect(resolveSendWorkspaceId({
      ...base,
      intent: 'new_chat',
      pendingWorkspaceId: 'ws-new',
      boundWorkspaceId: 'ws-old',
    })).toBe('ws-new')
  })

  it('后续轮次带会话已绑定项目（首轮漏绑时下一条消息自愈）', () => {
    expect(resolveSendWorkspaceId({
      ...base,
      intent: 'followup',
      boundWorkspaceId: 'ws-bound',
    })).toBe('ws-bound')
  })

  it('payload 显式给出的 workspaceId 优先（允许显式解绑为 null）', () => {
    expect(resolveSendWorkspaceId({
      ...base,
      hasWorkspaceField: true,
      payloadWorkspaceId: 'ws-explicit',
      intent: 'new_chat',
      pendingWorkspaceId: 'ws-pending',
      boundWorkspaceId: 'ws-bound',
    })).toBe('ws-explicit')
    expect(resolveSendWorkspaceId({
      ...base,
      hasWorkspaceField: true,
      payloadWorkspaceId: null,
      boundWorkspaceId: 'ws-bound',
    })).toBeNull()
  })

  it('无任何绑定时不臆造路径', () => {
    expect(resolveSendWorkspaceId({ ...base, intent: 'new_chat' })).toBeNull()
    expect(resolveSendWorkspaceId(base)).toBeNull()
  })
})
