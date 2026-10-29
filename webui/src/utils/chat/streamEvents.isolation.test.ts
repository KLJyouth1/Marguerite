import { describe, expect, it } from 'vitest'
import {
  acceptStreamSeq,
  frameSessionKey,
  isCurrentSessionPayload,
  isCurrentTaskPayload,
  isStaleEpoch,
  payloadSessionKey,
} from './streamEvents'

// PRD §2.2 ①「切换会话后旧会话的流式事件不得串扰」的可执行契约。这些闸门是
// 纯函数，被 ChatView 的每个 WS 事件处理器复用；它们一旦放宽，症状是「A 会话
// 的回复出现在 B 会话里」，而不是任何编译或类型错误。
describe('stream event session isolation', () => {
  const own = 'agent:main:web'
  const foreign = 'agent:other:web'

  it('drops events addressed to another session but stays lenient without a key', () => {
    expect(isCurrentSessionPayload({ session_key: own }, own)).toBe(true)
    expect(isCurrentSessionPayload({ session_key: foreign }, own)).toBe(false)
    // 未标会话键的帧（旧网关 / 侧车广播）按既有契约放行：拒绝它们会让实时输出
    // 整体消失。这条「宽松」是有意的，因此必须钉住，避免被当成漏洞收紧。
    expect(isCurrentSessionPayload({}, own)).toBe(true)
    // 草稿路由没有会话键时同样放行，否则首轮输出收不到。
    expect(isCurrentSessionPayload({ session_key: foreign }, '')).toBe(true)
  })

  it('reads the session key from any of the three spellings', () => {
    expect(payloadSessionKey({ key: own })).toBe(own)
    expect(payloadSessionKey({ session_key: own })).toBe(own)
    expect(payloadSessionKey({ sessionKey: own })).toBe(own)
    expect(payloadSessionKey({})).toBe('')
  })

  it('lets the WS envelope meta identity win over the payload key', () => {
    const frame = { session_key: foreign }
    expect(frameSessionKey(frame, { session: own })).toBe(own)
    // 空 meta / 无 meta 时回落到载荷自身的键，不能把「没声明」当成「不匹配」。
    expect(frameSessionKey(frame, {})).toBe(foreign)
    expect(frameSessionKey(frame)).toBe(foreign)
  })

  it('treats only an older epoch as stale', () => {
    expect(isStaleEpoch({ epoch: 2 }, 3)).toBe(true)
    expect(isStaleEpoch({ epoch: 3 }, 3)).toBe(false)
    expect(isStaleEpoch({ epoch: 4 }, 3)).toBe(false)
    // 旧网关不带 epoch：无从判断，必须放行（否则升级期实时流全断）。
    expect(isStaleEpoch({}, 3)).toBe(false)
    expect(isStaleEpoch({ epoch: Number.NaN }, 3)).toBe(false)
  })

  it('never advances the sequence cursor for a foreign session', () => {
    const decision = acceptStreamSeq({ session_key: foreign, stream_seq: 99 }, own, 5)
    expect(decision).toEqual({ accepted: false, nextStreamSeq: 5 })
    // 关键后果：游标没被外会话抬高，本会话的下一条真实事件仍然收得到。
    expect(acceptStreamSeq({ session_key: own, stream_seq: 6 }, own, decision.nextStreamSeq))
      .toEqual({ accepted: true, nextStreamSeq: 6 })
  })

  it('rejects duplicate and replayed sequences without moving the cursor', () => {
    expect(acceptStreamSeq({ stream_seq: 5 }, own, 5)).toEqual({ accepted: false, nextStreamSeq: 5 })
    expect(acceptStreamSeq({ stream_seq: 4 }, own, 5)).toEqual({ accepted: false, nextStreamSeq: 5 })
    expect(acceptStreamSeq({ stream_seq: 6 }, own, 5)).toEqual({ accepted: true, nextStreamSeq: 6 })
    // 无序号事件（推送型）放行，但不改游标。
    expect(acceptStreamSeq({}, own, 5)).toEqual({ accepted: true, nextStreamSeq: 5 })
  })

  it('drops a stale turn task but keeps untagged events', () => {
    expect(isCurrentTaskPayload({ task_id: 'task-old' }, 'task-new')).toBe(false)
    expect(isCurrentTaskPayload({ task_id: 'task-new' }, 'task-new')).toBe(true)
    expect(isCurrentTaskPayload({}, 'task-new')).toBe(true)
    expect(isCurrentTaskPayload({ task_id: 'task-old' }, '')).toBe(true)
  })
})
