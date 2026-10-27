import { describe, expect, it } from 'vitest'

import { isLastTurnSuccessful, resolveLastTurnStartTs } from './lastTurnStart'
import type { ChatRenderedMessage } from '@/types/chat'

type Row = Pick<ChatRenderedMessage, 'displayRole' | 'ts'> & {
  stopNotice?: boolean
  terminalFailure?: boolean
  interrupted?: boolean
}

function rows(list: Row[]): ChatRenderedMessage[] {
  return list as unknown as ChatRenderedMessage[]
}

describe('resolveLastTurnStartTs', () => {
  it('返回最近一条助手回复之前的用户消息时间戳', () => {
    expect(resolveLastTurnStartTs(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
    ]))).toBe(100)
  })

  it('多轮时只看最后一个回合', () => {
    expect(resolveLastTurnStartTs(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
      { displayRole: 'user', ts: 200 },
      { displayRole: 'assistant', ts: 201 },
    ]))).toBe(200)
  })

  it('没有助手回复、或下一条用户消息已发出但尚无回复时不显示', () => {
    expect(resolveLastTurnStartTs(rows([{ displayRole: 'user', ts: 100 }]))).toBeNull()
    expect(resolveLastTurnStartTs(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
      { displayRole: 'user', ts: 200 },
    ]))).toBeNull()
  })

  it('停止提示行不算本回合的完成回复', () => {
    expect(resolveLastTurnStartTs(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
      { displayRole: 'assistant', ts: 102, stopNotice: true },
    ]))).toBe(100)
  })

  it('流式生成 / 首屏 / 分享 / 分叉过渡期间一律不显示', () => {
    const list = rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
    ])
    expect(resolveLastTurnStartTs(list, { blocked: true })).toBeNull()
  })

  it('用户消息缺时间戳时返回 null（不猜窗口左界）', () => {
    expect(resolveLastTurnStartTs(rows([
      { displayRole: 'user', ts: null },
      { displayRole: 'assistant', ts: 101 },
    ]))).toBeNull()
  })
})

describe('isLastTurnSuccessful', () => {
  it('正常收尾的回合算成功', () => {
    expect(isLastTurnSuccessful(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
    ]))).toBe(true)
  })

  it('被中断或终止性失败的回合不算成功', () => {
    expect(isLastTurnSuccessful(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101, interrupted: true },
    ]))).toBe(false)
    expect(isLastTurnSuccessful(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101, terminalFailure: true },
    ]))).toBe(false)
  })

  it('停止提示行不参与判定，回看最后一条真实回复', () => {
    expect(isLastTurnSuccessful(rows([
      { displayRole: 'user', ts: 100 },
      { displayRole: 'assistant', ts: 101 },
      { displayRole: 'assistant', ts: 102, stopNotice: true },
    ]))).toBe(true)
  })

  it('还没有助手回复时不算成功', () => {
    expect(isLastTurnSuccessful(rows([
      { displayRole: 'user', ts: 100 },
    ]))).toBe(false)
  })
})
