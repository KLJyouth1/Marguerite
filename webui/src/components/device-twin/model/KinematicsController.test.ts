import { describe, expect, it } from 'vitest'
import {
  createKinematicsController,
  DOOR_SAFE_ANGLE,
} from './KinematicsController'

describe('KinematicsController — 动作联锁', () => {
  it('门关着时抽屉拉不出', () => {
    const k = createKinematicsController()
    const r = k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 })
    expect(r.ok).toBe(false)
    expect(r.reason).toBe('door-not-open-enough')
  })

  it('开门到安全角以上才能拉抽屉', () => {
    const k = createKinematicsController()
    k.apply({ kind: 'door', group: 'upperDoor', target: 1 })
    // 门开度是缓动目标值，apply 直接设 target=1，所以 door 已是 1
    const r = k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 })
    expect(r.ok).toBe(true)
  })

  it('抽屉没收回时门关不上', () => {
    const k = createKinematicsController()
    k.apply({ kind: 'door', group: 'upperDoor', target: 1 })
    k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 })
    // 现在尝试关门
    const r = k.apply({ kind: 'door', group: 'upperDoor', target: 0 })
    expect(r.ok).toBe(false)
    expect(r.reason).toBe('drawer-not-retracted')
  })

  it('先收抽屉再关门', () => {
    const k = createKinematicsController()
    k.apply({ kind: 'door', group: 'upperDoor', target: 1 })
    k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 })
    k.apply({ kind: 'drawer', group: 'upperDrawer', target: 0 })
    const r = k.apply({ kind: 'door', group: 'upperDoor', target: 0 })
    expect(r.ok).toBe(true)
  })

  it('prepareForExplode 自动收抽屉关门并锁定', () => {
    const k = createKinematicsController()
    k.apply({ kind: 'door', group: 'upperDoor', target: 1 })
    k.apply({ kind: 'door', group: 'lowerDoor', target: 1 })
    k.apply({ kind: 'drawer', group: 'lowerDrawer', target: 1 })
    const actions = k.prepareForExplode()
    expect(actions.length).toBeGreaterThanOrEqual(3)
    expect(k.snapshot().locked).toBe(true)
    // 锁定后任何动作被拒
    expect(k.apply({ kind: 'door', group: 'upperDoor', target: 0 }).ok).toBe(false)
    expect(k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 }).ok).toBe(false)
  })

  it('unlock 后恢复操作', () => {
    const k = createKinematicsController()
    k.prepareForExplode()
    k.unlock()
    expect(k.snapshot().locked).toBe(false)
    expect(k.apply({ kind: 'door', group: 'upperDoor', target: 1 }).ok).toBe(true)
  })

  it('DOOR_SAFE_ANGLE 在 (0,1) 之间', () => {
    expect(DOOR_SAFE_ANGLE).toBeGreaterThan(0)
    expect(DOOR_SAFE_ANGLE).toBeLessThan(1)
  })

  it('tick 缓动收敛到目标值，无 NaN', () => {
    const k = createKinematicsController()
    k.apply({ kind: 'door', group: 'upperDoor', target: 1 })
    let last = k.snapshot().doors.upperDoor
    for (let i = 0; i < 200; i++) {
      const s = k.tick(0.016)
      expect(Number.isFinite(s.doors.upperDoor)).toBe(true)
      expect(s.doors.upperDoor).toBeGreaterThanOrEqual(last - 0.0001)
      last = s.doors.upperDoor
    }
    expect(last).toBeGreaterThan(0.95)
  })

  it('不同抽屉对应不同门（上抽屉看上门，下抽屉看下门）', () => {
    const k = createKinematicsController()
    // 只开下门，上抽屉仍拉不出
    k.apply({ kind: 'door', group: 'lowerDoor', target: 1 })
    expect(k.apply({ kind: 'drawer', group: 'upperDrawer', target: 1 }).ok).toBe(false)
    // 下抽屉可以拉出
    expect(k.apply({ kind: 'drawer', group: 'lowerDrawer', target: 1 }).ok).toBe(true)
  })
})
