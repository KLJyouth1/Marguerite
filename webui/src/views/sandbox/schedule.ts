/**
 * 微缩沙盘时刻表 —— 纯函数：运行图驱动的速度曲线 / 位置 / 相位。
 *
 * 设计要点（对应任务书 B §1.3 / §2）：
 * - 一个 294s 的闭环运行图周期：始发站 A 发车 → 巡航（含越江桥）→ 制动 → 终到站 B
 *   → 停站开门 → 发车 → 巡航 → 制动 → 回到 A。
 * - 速度严格按区段：停站速度 == 0（开门乘降）、起步匀加速、巡航恒定、进站匀减速到 0。
 *   绝不匀速绕圈。
 * - **State From Time**：位置/速度/相位是全局时间 t 的纯函数，没有任何内部累积态。
 *   时间轴拖回同一时刻，distanceAt/speedAt/phaseAt 给出完全一致的结果（固定种子可回放）。
 * - 3 列车按周期 1/3 错开（offset = k * 294/3），保持闭塞间隔。
 *
 * 本模块不依赖 three / DOM / Math.random —— 可在 node 单测中直接跑。
 */

/** 一个运行图周期（秒）。任务书指定 294s（始发/越江/终到/发车/到发）。 */
export const LOOP_PERIOD = 294

/** 巡航速度（弧长单位/秒）。停站为 0，加减速在 0 与它之间线性过渡。 */
export const CRUISE_SPEED = 1.0

/** 闭环弧长（弧长单位）。由各区段三角形/矩形面积积分得出，见 SEGMENTS。 */
export const LOOP_LENGTH = 194.5

/** 列车数（任务书：3 列压缩编组）。 */
export const TRAIN_COUNT = 3

export type StationId = 'A' | 'B'

/** 列车运行相位：停站 / 出站加速 / 巡航 / 进站减速。 */
export type TrainPhase = 'dwell' | 'departing' | 'cruise' | 'arriving'

type SegmentKind = 'dwell' | 'accel' | 'cruise' | 'brake'

interface Segment {
  /** 区段起始秒（含） */
  t0: number
  /** 区段结束秒（不含） */
  t1: number
  kind: SegmentKind
  /** 停站区段停靠的车站；非停站为 null */
  station: StationId | null
  /** 区段起点处的闭环弧长 s */
  s0: number
  /** 区段终点处的闭环弧长 s */
  s1: number
}

/**
 * 运行图区段表（时间与弧长成对）。
 *
 * 时间合计 = 18+20+80+15+25+20+76+22+18 = 294。
 * 弧长合计（加/减速三角形 + 巡航矩形）：
 *   accel1 0.5·20 = 10, cruise1 80, brake1 0.5·15 = 7.5  → 站 B 于 s=97.5
 *   accel2 0.5·20 = 10, cruise2 76, brake2 0.5·22 = 11 → 回到 s=194.5 ≡ 站 A
 */
export const SEGMENTS: readonly Segment[] = [
  { t0: 0,   t1: 18,  kind: 'dwell', station: 'A', s0: 0,    s1: 0 },
  { t0: 18,  t1: 38,  kind: 'accel', station: null, s0: 0,    s1: 10 },
  { t0: 38,  t1: 118, kind: 'cruise', station: null, s0: 10,   s1: 90 },
  { t0: 118, t1: 133, kind: 'brake',  station: 'B', s0: 90,   s1: 97.5 },
  { t0: 133, t1: 158, kind: 'dwell', station: 'B', s0: 97.5, s1: 97.5 },
  { t0: 158, t1: 178, kind: 'accel', station: null, s0: 97.5, s1: 107.5 },
  { t0: 178, t1: 254, kind: 'cruise', station: null, s0: 107.5, s1: 183.5 },
  { t0: 254, t1: 276, kind: 'brake',  station: 'A', s0: 183.5, s1: 194.5 },
  { t0: 276, t1: 294, kind: 'dwell', station: 'A', s0: 194.5, s1: 194.5 },
]

/** 站点在闭环上的弧长位置（A ≡ 0 ≡ L；B 于中段）。 */
export const STATION_ARC: Readonly<Record<StationId, number>> = {
  A: 0,
  B: 97.5,
}

/** 把任意时间（可为负 / 远超一个周期）折叠进 [0, LOOP_PERIOD)。 */
export function wrapTime(t: number): number {
  const m = ((t % LOOP_PERIOD) + LOOP_PERIOD) % LOOP_PERIOD
  // 边界：浮点误差可能让 294-ε 落回 0 附近，保持闭区间 [0, period) 稳定
  return m < 0 ? 0 : m
}

/** 找到 t 落入的区段（t 已折叠）。 */
function segmentAt(tt: number): { seg: Segment; index: number } {
  for (let i = 0; i < SEGMENTS.length; i++) {
    const seg = SEGMENTS[i]
    if (tt >= seg.t0 && tt < seg.t1) return { seg, index: i }
  }
  // 浮点兜底：tt 恰好等于周期末 → 归入最后一个停站段
  const last = SEGMENTS[SEGMENTS.length - 1]
  return { seg: last, index: SEGMENTS.length - 1 }
}

/** 区段内速度：停站严格 0；加速线性升到巡航；巡航恒定；制动线性降到 0。 */
export function speedAt(t: number): number {
  const tt = wrapTime(t)
  const { seg } = segmentAt(tt)
  const dur = seg.t1 - seg.t0
  const elapsed = tt - seg.t0
  switch (seg.kind) {
    case 'dwell':
      return 0
    case 'accel':
      return CRUISE_SPEED * (elapsed / dur)
    case 'cruise':
      return CRUISE_SPEED
    case 'brake':
      return CRUISE_SPEED * (1 - elapsed / dur)
  }
}

/**
 * 区段内闭环弧长 s（0 ≤ s < LOOP_LENGTH）。
 * 与速度曲线是积分关系：加速/制动段为三角形面积，巡航段为矩形。
 */
export function distanceAt(t: number): number {
  const tt = wrapTime(t)
  const { seg } = segmentAt(tt)
  const dur = seg.t1 - seg.t0
  const elapsed = tt - seg.t0
  let s: number
  switch (seg.kind) {
    case 'dwell':
      s = seg.s0
      break
    case 'accel':
      // ∫ CRUISE·(e/dur) de = CRUISE·elapsed² / (2·dur)
      s = seg.s0 + (CRUISE_SPEED * elapsed * elapsed) / (2 * dur)
      break
    case 'cruise':
      s = seg.s0 + CRUISE_SPEED * elapsed
      break
    case 'brake':
      // ∫ CRUISE·(1 - e/dur) de = CRUISE·(elapsed - elapsed²/(2·dur))
      s = seg.s0 + CRUISE_SPEED * (elapsed - (elapsed * elapsed) / (2 * dur))
      break
  }
  return s % LOOP_LENGTH
}

/** 区段 → 对外相位（停站/出站/巡航/进站）。 */
function phaseOf(seg: Segment): TrainPhase {
  switch (seg.kind) {
    case 'dwell':
      return 'dwell'
    case 'accel':
      return 'departing'
    case 'cruise':
      return 'cruise'
    case 'brake':
      return 'arriving'
  }
}

export interface TrainState {
  /** 全局周期内的折叠时间 */
  t: number
  phase: TrainPhase
  /** 瞬时速度（弧长/秒）；停站时为 0 */
  speed: number
  /** 闭环弧长 0..LOOP_LENGTH */
  distance: number
  /** 正停靠的车站；运行中为 null */
  station: StationId | null
  /** 车轮走过的弧长（含每圈累积，用于轮子转动角度，不折叠） */
  wheelDistance: number
}

/**
 * 某列车在全局时间 t 的完整状态（纯函数）。
 * @param trainIndex 0..TRAIN_COUNT-1，三列车按周期 1/3 错开
 * @param t 全局沙盘时间（秒）
 */
export function trainStateAt(trainIndex: number, t: number): TrainState {
  const offset = (trainIndex * LOOP_PERIOD) / TRAIN_COUNT
  const local = t + offset
  const tt = wrapTime(local)
  const { seg } = segmentAt(tt)
  const distance = distanceAt(local)
  // wheelDistance：含圈数累积。每列车 offset 不同但同速曲线，直接用未折叠的本地时间积分
  // 会因 wrap 断圈而跳变，所以用 已完成圈数*L + 当圈 distance。
  const completedLoops = Math.floor(local / LOOP_PERIOD)
  const wheelDistance = completedLoops * LOOP_LENGTH + distance
  return {
    t: tt,
    phase: phaseOf(seg),
    speed: speedAt(local),
    distance,
    station: seg.kind === 'dwell' ? seg.station : null,
    wheelDistance,
  }
}

/**
 * 紧随当前区段之后的状态（“下一步做什么”）。纯函数。
 * 若当前是最后一区段，下一步绕回第一区段（闭环）。
 */
export function nextTrainState(trainIndex: number, t: number): TrainState {
  const offset = (trainIndex * LOOP_PERIOD) / TRAIN_COUNT
  const local = t + offset
  const tt = wrapTime(local)
  const { index } = segmentAt(tt)
  const nextIndex = (index + 1) % SEGMENTS.length
  const next = SEGMENTS[nextIndex]
  // 下一步的代表时刻：取下一区段的起点
  const nextRep = local + (next.t0 - tt)
  return trainStateAt(trainIndex, nextRep)
}

/** 供测试/UI 直接断言的停站时间窗（秒，周期内）。 */
export function dwellWindows(): ReadonlyArray<{ station: StationId; start: number; end: number }> {
  return SEGMENTS.filter((s) => s.kind === 'dwell').map((s) => ({
    station: s.station as StationId,
    start: s.t0,
    end: s.t1,
  }))
}
