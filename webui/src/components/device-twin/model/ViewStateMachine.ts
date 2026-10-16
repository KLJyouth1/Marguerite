/**
 * ViewStateMachine — 六态视图状态机（纯逻辑，不依赖 three/DOM，node 可测）。
 *
 * 六态：product | open | explode | section | cycle | inspect
 *
 * 状态切换不瞬移：渲染层拿到 0..1 的 progress，用 `1 - exp(-dt*speed)` 缓动
 * 收敛到目标值。本类只管「允许去哪、当前在哪个态、progress 怎么推进」，
 * 不碰 Mesh。
 *
 * 迁移约束（与 KinematicsController 配合，但本类独立可测）：
 * - inspect 是瞬时态：选中某部件 → inspect；点空白 → 回到进入 inspect 前的态。
 * - explode 进入前由 KinematicsController 自动收抽屉关门（本类只做状态迁移，
 *   联锁是 KinematicsController 的职责 —— 两者通过 requestTransition 的 precondition 沟通）。
 */

export type DeviceViewState =
  | 'product'   // 整机闭合
  | 'open'      // 开门
  | 'explode'   // 爆炸
  | 'section'   // 剖面
  | 'cycle'     // 工作循环（流体粒子动起来）
  | 'inspect'   // 部件特写

export const VIEW_STATES: readonly DeviceViewState[] = [
  'product', 'open', 'explode', 'section', 'cycle', 'inspect',
] as const

/** 每个态的缓动速度（越大越快收敛；禁止瞬移） */
export const VIEW_SPEED: Record<DeviceViewState, number> = {
  product: 4.5,
  open: 3.5,
  explode: 2.8,
  section: 3.0,
  cycle: 5.0,
  inspect: 6.0,
}

/**
 * 合法迁移表。inspect 不写在表里 —— 它是「从任意态进入、再回到来源态」的覆盖层，
 * 由 enterInspect / exitInspect 单独处理。
 */
const TRANSITIONS: ReadonlySet<string> = new Set([
  'product->open',
  'open->product',
  'product->explode',
  'open->explode',
  'explode->product',
  'product->section',
  'section->product',
  'product->cycle',
  'cycle->product',
  'open->cycle',
  'cycle->open',
  'explode->open',
  'section->open',
])

export interface ViewStateSnapshot {
  state: DeviceViewState
  /** 进入 inspect 前的态（退出时回退） */
  inspectReturnTo: DeviceViewState | null
  /** 0..1 缓动进度（渲染层用它插值变换） */
  progress: number
  /** 当前态目标 progress = 1 */
  targetProgress: number
}

export function createViewStateMachine(): ViewStateMachine {
  let state: DeviceViewState = 'product'
  let inspectReturnTo: DeviceViewState | null = null
  let progress = 1
  let target = 1

  function canTransition(to: DeviceViewState): boolean {
    if (to === state) return false
    if (to === 'inspect') return false // 走 enterInspect
    const key = `${state}->${to}`
    return TRANSITIONS.has(key)
  }

  function requestTransition(to: DeviceViewState): { ok: boolean; reason?: string } {
    if (to === state) return { ok: false, reason: 'already-in-state' }
    if (to === 'inspect') return { ok: false, reason: 'use-enterInspect' }
    if (inspectReturnTo) return { ok: false, reason: 'in-inspect-exit-first' }
    if (!canTransition(to)) {
      return { ok: false, reason: `illegal-transition ${state}->${to}` }
    }
    state = to
    // 切态时 progress 从 0 缓动到 1（渲染层据此插值）。
    progress = 0
    target = 1
    return { ok: true }
  }

  function enterInspect(partId: string): { ok: boolean; reason?: string } {
    if (inspectReturnTo) return { ok: false, reason: 'already-inspecting' }
    inspectReturnTo = state
    state = 'inspect'
    target = 1
    void partId // partId 由渲染层/控制器持有，状态机不存
    return { ok: true }
  }

  function exitInspect(): { ok: boolean; returnedTo: DeviceViewState } {
    if (!inspectReturnTo) return { ok: false, returnedTo: state }
    const back = inspectReturnTo
    inspectReturnTo = null
    state = back
    target = 1
    return { ok: true, returnedTo: back }
  }

  /**
   * 推进缓动。返回新的 progress（不突变）。
   * `1 - exp(-dt * speed)`：指数收敛，首帧快、尾帧慢，无跳变。
   */
  function tick(dt: number): number {
    if (!Number.isFinite(dt) || dt <= 0) return progress
    const speed = VIEW_SPEED[state]
    progress += (target - progress) * (1 - Math.exp(-dt * speed))
    // 钳位 + 收敛判定（避免浮点尾差永远不到 1）
    if (target - progress < 0.001) progress = target
    if (!Number.isFinite(progress)) progress = target
    return progress
  }

  function snapshot(): ViewStateSnapshot {
    return { state, inspectReturnTo, progress, targetProgress: target }
  }

  return {
    requestTransition,
    enterInspect,
    exitInspect,
    tick,
    snapshot,
    get state() { return state },
    get inspectReturnedTo() { return inspectReturnTo },
  }
}

export interface ViewStateMachine {
  requestTransition(to: DeviceViewState): { ok: boolean; reason?: string }
  enterInspect(partId: string): { ok: boolean; reason?: string }
  exitInspect(): { ok: boolean; returnedTo: DeviceViewState }
  tick(dt: number): number
  snapshot(): ViewStateSnapshot
  readonly state: DeviceViewState
  readonly inspectReturnedTo: DeviceViewState | null
}
