import { describe, expect, it } from 'vitest'
import { WAITING_HINT_AFTER_SEC, waitingSerialHint } from './waitingHint'

// 文案用假 t 直接回显参数，断言只关心「该不该提示 / 参数对不对」，
// 不把 6 个 locale 的具体措辞写进单测（措辞会改，阈值语义不该改）。
const t = (key: string, params: Record<string, unknown>) => `${key}|${String(params.s)}`

describe('waitingSerialHint', () => {
  it('未流式 / 已有输出时不提示（避免在模型已经在输出时还说"在等"）', () => {
    expect(waitingSerialHint({ streaming: false, hasVisibleOutput: false, elapsedSec: 99, translate: t })).toBe('')
    expect(waitingSerialHint({ streaming: true, hasVisibleOutput: true, elapsedSec: 99, translate: t })).toBe('')
  })

  it('阈值以下不提示，达到阈值才提示，并带上真实秒数', () => {
    const below = { streaming: true, hasVisibleOutput: false, translate: t }
    expect(waitingSerialHint({ ...below, elapsedSec: WAITING_HINT_AFTER_SEC - 1 })).toBe('')
    expect(waitingSerialHint({ ...below, elapsedSec: WAITING_HINT_AFTER_SEC })).toBe(
      `chat.stream.waitingSerialHint|${WAITING_HINT_AFTER_SEC}`,
    )
    expect(waitingSerialHint({ ...below, elapsedSec: 27 })).toBe('chat.stream.waitingSerialHint|27')
  })

  it('秒数取整并夹到非负（时钟回拨也不能出现 "-3s"）', () => {
    const base = { streaming: true, hasVisibleOutput: false, translate: t, afterSec: 0 }
    expect(waitingSerialHint({ ...base, elapsedSec: 12.7 })).toBe('chat.stream.waitingSerialHint|12')
    expect(waitingSerialHint({ ...base, elapsedSec: -3 })).toBe('chat.stream.waitingSerialHint|0')
  })
})
