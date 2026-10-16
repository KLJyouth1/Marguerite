import { describe, expect, it } from 'vitest'
import {
  CLICK_MOVE_THRESHOLD_PX,
  exceedsDragThreshold,
  isClickTap,
  trainIndexFromEntityId,
} from './pickLogic'

describe('点击位移阈值判定', () => {
  it('完全没动 = 点击', () => {
    expect(isClickTap({ x: 100, y: 100 }, { x: 100, y: 100 })).toBe(true)
  })

  it('位移在阈值内（≤5px）= 点击', () => {
    expect(isClickTap({ x: 100, y: 100 }, { x: 103, y: 104 })).toBe(true)
    // 刚好踩在阈值上（勾股 sqrt(25)=5）
    expect(isClickTap({ x: 100, y: 100 }, { x: 105, y: 100 })).toBe(true)
  })

  it('位移超出阈值 = 拖拽（不做点选）', () => {
    expect(isClickTap({ x: 100, y: 100 }, { x: 106, y: 0 })).toBe(false)
    expect(isClickTap({ x: 100, y: 100 }, { x: 104, y: 4 })).toBe(false)
  })

  it('exceedsDragThreshold 与 isClickTap 互为反义', () => {
    const down = { x: 50, y: 50 }
    expect(exceedsDragThreshold(down, { x: 52, y: 51 })).toBe(false)
    expect(exceedsDragThreshold(down, { x: 60, y: 50 })).toBe(true)
  })

  it('阈值常量就是任务书要求的 ~5px', () => {
    expect(CLICK_MOVE_THRESHOLD_PX).toBe(5)
  })
})

describe('命中 → 列车下标 / 空白取消', () => {
  it('"train:0" → 0, "train:2" → 2', () => {
    expect(trainIndexFromEntityId('train:0')).toBe(0)
    expect(trainIndexFromEntityId('train:2')).toBe(2)
  })

  it('空白处点击 / 未命中 → null（调用方据此取消选中）', () => {
    expect(trainIndexFromEntityId(undefined)).toBeNull()
    expect(trainIndexFromEntityId(null)).toBeNull()
    expect(trainIndexFromEntityId(42)).toBeNull()
  })

  it('非列车的 entityId（站台/桥/桌子）→ null，不误选', () => {
    expect(trainIndexFromEntityId('station:A')).toBeNull()
    expect(trainIndexFromEntityId('bridge')).toBeNull()
    expect(trainIndexFromEntityId('train:')).toBeNull()
    expect(trainIndexFromEntityId('train:abc')).toBeNull()
  })
})
