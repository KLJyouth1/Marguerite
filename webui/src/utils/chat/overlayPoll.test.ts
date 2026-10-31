import { describe, expect, it } from 'vitest'
import {
  OVERLAY_ACTIVE_POLL_MS,
  OVERLAY_IDLE_POLL_MS,
  nextOverlayPollMs,
} from './overlayPoll'

describe('nextOverlayPollMs（R-17 悬浮球兜底轮询节奏）', () => {
  it('空闲且已连接时放慢，不为「还是 0」付 15 秒一次的代价', () => {
    expect(nextOverlayPollMs(false, true)).toBe(OVERLAY_IDLE_POLL_MS)
  })

  it('有任务在跑时保持原频率 —— 数量是用户唯一看的东西', () => {
    expect(nextOverlayPollMs(true, true)).toBe(OVERLAY_ACTIVE_POLL_MS)
  })

  it('未连接时不许放慢：那正是要靠轮询发现网关回来的时刻', () => {
    expect(nextOverlayPollMs(false, false)).toBe(OVERLAY_ACTIVE_POLL_MS)
    expect(nextOverlayPollMs(true, false)).toBe(OVERLAY_ACTIVE_POLL_MS)
  })

  it('空闲档必须比运行档更慢，但不许等于「停掉」', () => {
    expect(OVERLAY_IDLE_POLL_MS).toBeGreaterThan(OVERLAY_ACTIVE_POLL_MS)
    expect(OVERLAY_IDLE_POLL_MS).toBeLessThan(5 * 60_000)
  })
})
