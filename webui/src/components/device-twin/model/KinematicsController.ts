/**
 * KinematicsController — 动作联锁控制器（纯逻辑，node 可测）。
 *
 * 铁律：禁止按钮直改 Mesh。所有运动请求经这里，由统一状态（door angle / drawer pull）
 * 收敛后渲染层读取。联锁规则：
 *
 * 1. **抽屉拉出前必须先开门到安全角** —— 门关着抽屉会撞门框。
 * 2. **关门前先收抽屉** —— 抽屉没收回门会撞抽屉。
 * 3. **进爆炸/剖面前自动收抽屉关门** —— 爆炸视图里抽屉弹出会乱套。
 * 4. **爆炸中禁止不合理操作** —— 爆炸态下抽屉/门请求一律拒绝。
 *
 * 本类与 ViewStateMachine 解耦：它只管「运动组的 0..1 开度」，
 * 状态机切换前由上层调用 `prepareForExplode()` / `prepareForSection()` 做自动复位。
 */

import type { KinematicGroup } from './PartNode'

export type ActuatorRequest =
  | { kind: 'door'; group: 'upperDoor' | 'lowerDoor'; target: 0 | 1 }
  | { kind: 'drawer'; group: 'upperDrawer' | 'lowerDrawer'; target: 0 | 1 }

export interface KinematicsState {
  /** 各门 0..1 开度（0=关，1=全开） */
  doors: Record<'upperDoor' | 'lowerDoor', number>
  /** 各抽屉 0..1 拉出量（0=收回，1=全拉出） */
  drawers: Record<'upperDrawer' | 'lowerDrawer', number>
  /** 联锁是否挂起（爆炸/剖面态下锁定） */
  locked: boolean
}

/** 抽屉能拉出的最小门开角（0..1）。低于此角拒绝拉出。 */
export const DOOR_SAFE_ANGLE = 0.45

/** 抽屉收回的判定阈值（高于此值算「没收回」） */
export const DRAWER_RETRACTED = 0.05

function clamp01(v: number): number {
  if (!Number.isFinite(v)) return 0
  return v < 0 ? 0 : v > 1 ? 1 : v
}

export function createKinematicsController() {
  const state: KinematicsState = {
    doors: { upperDoor: 0, lowerDoor: 0 },
    drawers: { upperDrawer: 0, lowerDrawer: 0 },
    locked: false,
  }

  /** 所有门是否都已关到接近闭合 */
  function allDoorsClosed(): boolean {
    return state.doors.upperDoor < DRAWER_RETRACTED && state.doors.lowerDoor < DRAWER_RETRACTED
  }

  /** 所有抽屉是否都已收回 */
  function allDrawersRetracted(): boolean {
    return state.drawers.upperDrawer < DRAWER_RETRACTED && state.drawers.lowerDrawer < DRAWER_RETRACTED
  }

  /** 某扇门是否开到了抽屉安全角以上 */
  function doorOpenEnoughFor(drawer: 'upperDrawer' | 'lowerDrawer'): boolean {
    // 上层抽屉看上门，下层抽屉看下门
    const doorGroup = drawer === 'upperDrawer' ? 'upperDoor' : 'lowerDoor'
    return state.doors[doorGroup] >= DOOR_SAFE_ANGLE
  }

  function applyDoor(req: Extract<ActuatorRequest, { kind: 'door' }>): { ok: boolean; reason?: string } {
    if (state.locked) return { ok: false, reason: 'kinematics-locked' }
    // 关门（target=0）前必须先收对应抽屉
    if (req.target === 0) {
      const drawerGroup: 'upperDrawer' | 'lowerDrawer' =
        req.group === 'upperDoor' ? 'upperDrawer' : 'lowerDrawer'
      if (state.drawers[drawerGroup] > DRAWER_RETRACTED) {
        return { ok: false, reason: 'drawer-not-retracted' }
      }
    }
    state.doors[req.group] = req.target
    return { ok: true }
  }

  function applyDrawer(req: Extract<ActuatorRequest, { kind: 'drawer' }>): { ok: boolean; reason?: string } {
    if (state.locked) return { ok: false, reason: 'kinematics-locked' }
    // 拉出（target=1）前必须先开门到安全角
    if (req.target === 1 && !doorOpenEnoughFor(req.group)) {
      return { ok: false, reason: 'door-not-open-enough' }
    }
    state.drawers[req.group] = req.target
    return { ok: true }
  }

  function apply(req: ActuatorRequest): { ok: boolean; reason?: string } {
    return req.kind === 'door' ? applyDoor(req) : applyDrawer(req)
  }

  /** 进爆炸/剖面前自动复位：关所有门、收所有抽屉。返回复位清单。 */
  function prepareForExplode(): ReadonlyArray<{ group: KinematicGroup; action: 'close' | 'retract' }> {
    state.locked = true
    const actions: { group: KinematicGroup; action: 'close' | 'retract' }[] = []
    for (const g of ['upperDoor', 'lowerDoor'] as const) {
      if (state.doors[g] > DRAWER_RETRACTED) {
        state.doors[g] = 0
        actions.push({ group: g, action: 'close' })
      }
    }
    for (const g of ['upperDrawer', 'lowerDrawer'] as const) {
      if (state.drawers[g] > DRAWER_RETRACTED) {
        state.drawers[g] = 0
        actions.push({ group: g, action: 'retract' })
      }
    }
    return actions
  }

  /** 退出爆炸/剖面后解锁，允许正常操作 */
  function unlock(): void {
    state.locked = false
  }

  /**
   * 缓动推进（与 ViewStateMachine 同一套指数收敛）。
   * 渲染层每帧调用，传入 dt，返回新的开度值。
   */
  function tick(dt: number): KinematicsState {
    if (!Number.isFinite(dt) || dt <= 0) return snapshot()
    const speed = 5.0
    const damp = 1 - Math.exp(-dt * speed)
    for (const g of ['upperDoor', 'lowerDoor'] as const) {
      const target = state.doors[g] // 已是 0/1 目标值
      state.doors[g] = clamp01(state.doors[g] + (target - state.doors[g]) * damp)
    }
    for (const g of ['upperDrawer', 'lowerDrawer'] as const) {
      const target = state.drawers[g]
      state.drawers[g] = clamp01(state.drawers[g] + (target - state.drawers[g]) * damp)
    }
    return snapshot()
  }

  function snapshot(): KinematicsState {
    return {
      doors: { ...state.doors },
      drawers: { ...state.drawers },
      locked: state.locked,
    }
  }

  return {
    apply,
    prepareForExplode,
    unlock,
    tick,
    snapshot,
    allDoorsClosed,
    allDrawersRetracted,
  }
}

export type KinematicsController = ReturnType<typeof createKinematicsController>
