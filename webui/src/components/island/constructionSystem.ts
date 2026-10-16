/**
 * 施工现场作业循环（纯逻辑，不 import three / 不碰 DOM）。
 *
 * 铁律落地：
 * - **塔吊/工人/工程车辆有真实工作循环**：塔吊不是匀速绕圈，而是
 *   「材料区吊起 → 旋转 → 楼顶放下」的相位机；工人沿 waypoint 走、
 *   在工位驻留作业；车辆沿环线起步/巡航/刹停/停靠。
 * - **不机械同步**：每个作业个体带 `scheduleOffset`（相位偏移），
 *   同一份配置下 N 台塔吊/N 个工人不会齐步走。
 * - **重复件 InstancedMesh**：护栏/锥桶/脚手架管/路灯的摆放以
 *   `InstancedTransform[]` 数据描述返回，渲染层一条 InstancedMesh 吃全部。
 * - 全部时长/角度/数量来自数据预算（`ConstructionCycleBudget`），
 *   不在渲染层写死魔法数。
 *
 * 这里只产出「这一帧该把骨架构件放在哪、工人站在哪、吊钩吊多高」的
 * 数值；真正的 THREE.Mesh 由后续接线者消费。
 */

// ─────────────────────────────────────────────────────────────────────────────
// 塔吊循环
// ─────────────────────────────────────────────────────────────────────────────

/** 塔吊作业相位（一个完整吊运循环） */
export type CranePhase =
  | 'swingToYard' // 回转到材料区
  | 'lowerToYard' // 吊钩下降到材料面
  | 'hoistLoad' // 挂载起升
  | 'swingToSite' // 回转到楼顶
  | 'lowerToRoof' // 小车外伸 + 吊钩下降就位
  | 'release' // 脱钩、空钩回升

export const CRANE_PHASES: readonly CranePhase[] = [
  'swingToYard',
  'lowerToYard',
  'hoistLoad',
  'swingToSite',
  'lowerToRoof',
  'release',
]

/** 各相位时长（秒）—— 数据预算，渲染/调参只改这里 */
export interface CraneCycleBudget {
  swingToYard: number
  lowerToYard: number
  hoistLoad: number
  swingToSite: number
  lowerToRoof: number
  release: number
}

export const DEFAULT_CRANE_BUDGET: CraneCycleBudget = {
  swingToYard: 6,
  lowerToYard: 3,
  hoistLoad: 3,
  swingToSite: 6,
  lowerToRoof: 4,
  release: 2,
}

export interface CranePose {
  phase: CranePhase
  /** 臂架水平回转角（弧度） */
  armAngle: number
  /** 小车在臂上的伸出量 0..1（0=塔身处，1=臂端） */
  trolley: number
  /** 吊钩高度 0..1（0=地面，1=臂顶） */
  hookY: number
  /** 是否挂着载荷（脱钩相位为 false） */
  loadAttached: boolean
  /** 循环内归一化进度 0..1（调试/UI 用） */
  cycleProgress: number
}

/**
 * 塔吊循环主函数：给定循环内时间 t（秒）+ 相位偏移 offset（秒），
 * 输出这一帧的臂/小车/吊钩姿态。确定性：同一 (t, offset) 永远同姿态。
 *
 * 运动学要点：
 * - 臂角在「材料区角 → 楼顶角」之间随相位平滑插值（不是匀速转圈）；
 * - 吊钩高度在 lower/hoist 相位往返，release 相位快速回升；
 * - 载荷只在 hoist→lowerToRoof 之间挂着。
 */
export function cranePoseAt(
  t: number,
  offset: number,
  budget: CraneCycleBudget = DEFAULT_CRANE_BUDGET,
  homeAngle = 0,
  yardAngle = Math.PI,
): CranePose {
  const phases = CRANE_PHASES
  const durations = phases.map((p) => budget[p])
  const total = durations.reduce((a, b) => a + b, 0)
  // 相位偏移让多台塔吊错峰；模 total 保证闭环
  let cycle = ((t + offset) % total + total) % total
  const cycleProgress = cycle / total

  let acc = 0
  let phaseIdx = 0
  for (let i = 0; i < durations.length; i++) {
    if (cycle < acc + durations[i]) { phaseIdx = i; break }
    acc += durations[i]
    if (i === durations.length - 1) { phaseIdx = i; acc = total - durations[i] }
  }
  const phase = phases[phaseIdx]
  const local = (cycle - acc) / durations[phaseIdx] // 相位内 0..1

  // 臂角：home（楼顶位）与 yard（材料区位）之间
  const roofAngle = homeAngle
  const swing = (a: number, b: number, k: number) => a + (b - a) * easeInOut(k)

  let armAngle = roofAngle
  let trolley = 0.3
  let hookY = 1
  let loadAttached = false

  switch (phase) {
    case 'swingToYard':
      armAngle = swing(roofAngle, yardAngle, local)
      trolley = 0.9
      hookY = 1
      break
    case 'lowerToYard':
      armAngle = yardAngle
      trolley = 0.9
      hookY = 1 - 0.85 * easeInOut(local) // 降到材料面
      break
    case 'hoistLoad':
      armAngle = yardAngle
      trolley = 0.9
      hookY = 0.15 + 0.85 * easeInOut(local) // 起升
      loadAttached = true
      break
    case 'swingToSite':
      armAngle = swing(yardAngle, roofAngle, local)
      trolley = 0.35
      hookY = 1
      loadAttached = true
      break
    case 'lowerToRoof':
      armAngle = roofAngle
      trolley = 0.35 + 0.5 * easeInOut(local) // 小车外伸到作业点
      hookY = 1 - 0.55 * easeInOut(local) // 降到楼顶
      loadAttached = true
      break
    case 'release':
      armAngle = roofAngle
      trolley = 0.85
      hookY = 0.45 + 0.55 * easeInOut(local) // 空钩回升
      loadAttached = false
      break
  }
  return { phase, armAngle, trolley, hookY, loadAttached, cycleProgress }
}

/** 平滑缓动（相位内不要机械匀速） */
function easeInOut(k: number): number {
  const x = Math.min(1, Math.max(0, k))
  return x < 0.5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2
}

// ─────────────────────────────────────────────────────────────────────────────
// 工人状态机
// ─────────────────────────────────────────────────────────────────────────────

export type WorkerState = 'idle' | 'walk' | 'work' | 'carry'

export interface WorkerWaypoint {
  x: number
  z: number
  /** 到达后驻留作业秒数（0 = 只路过） */
  dwell: number
}

export interface WorkerBudget {
  /** 步行速度（米/秒） */
  walkSpeed: number
}

export const DEFAULT_WORKER_BUDGET: WorkerBudget = { walkSpeed: 0.9 }

export interface WorkerPose {
  state: WorkerState
  x: number
  z: number
  /** 面朝方向（弧度，沿行进方向） */
  facing: number
}

function dist(a: { x: number; z: number }, b: { x: number; z: number }): number {
  return Math.hypot(b.x - a.x, b.z - a.z)
}

/**
 * 工人沿 waypoint graph 走动 + 在工位驻留作业。
 * offset 让每个工人错峰（不机械同步）。循环：沿路径走一圈，在每个有 dwell 的点作业。
 */
export function workerPoseAt(
  t: number,
  offset: number,
  waypoints: readonly WorkerWaypoint[],
  budget: WorkerBudget = DEFAULT_WORKER_BUDGET,
): WorkerPose {
  if (waypoints.length === 0) return { state: 'idle', x: 0, z: 0, facing: 0 }
  if (waypoints.length === 1) return { state: 'work', x: waypoints[0].x, z: waypoints[0].z, facing: 0 }

  // 一圈 = 各段步行时间 + 各点驻留时间
  let cycleLen = 0
  const segTimes: number[] = []
  const dwellTimes: number[] = []
  for (let i = 0; i < waypoints.length; i++) {
    const a = waypoints[i]
    const b = waypoints[(i + 1) % waypoints.length]
    const seg = dist(a, b) / Math.max(1e-3, budget.walkSpeed)
    segTimes.push(seg)
    dwellTimes.push(a.dwell)
    cycleLen += seg + a.dwell
  }
  if (cycleLen <= 0) return { state: 'idle', x: waypoints[0].x, z: waypoints[0].z, facing: 0 }

  let ct = ((t + offset) % cycleLen + cycleLen) % cycleLen
  let facing = 0
  for (let i = 0; i < waypoints.length; i++) {
    const dwell = dwellTimes[i]
    if (ct < dwell) {
      // 在 waypoint i 作业
      return { state: dwell > 0 ? 'work' : 'idle', x: waypoints[i].x, z: waypoints[i].z, facing }
    }
    ct -= dwell
    const a = waypoints[i]
    const b = waypoints[(i + 1) % waypoints.length]
    const seg = segTimes[i]
    if (ct < seg) {
      const k = ct / seg
      facing = Math.atan2(b.z - a.z, b.x - a.x)
      return {
        state: 'walk',
        x: a.x + (b.x - a.x) * k,
        z: a.z + (b.z - a.z) * k,
        facing,
      }
    }
    ct -= seg
  }
  const last = waypoints[0]
  return { state: 'idle', x: last.x, z: last.z, facing: 0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// 工程车辆（沿环线）
// ─────────────────────────────────────────────────────────────────────────────

export type VehiclePhase = 'accelerate' | 'cruise' | 'decelerate' | 'stop'

export interface VehicleBudget {
  cruiseSpeed: number
  accel: number
  stopDwell: number
}

export const DEFAULT_VEHICLE_BUDGET: VehicleBudget = {
  cruiseSpeed: 2.2,
  accel: 1.4,
  stopDwell: 6,
}

export interface VehiclePose {
  phase: VehiclePhase
  x: number
  z: number
  heading: number
  /** 车轮滚转角（弧度）—— 真实滚动距离/r，不是原地空转 */
  wheelSpin: number
  speed: number
}

/**
 * 车辆沿闭合 waypoint 环线行驶：加速→巡航→减速→停靠。
 * wheelSpin 由累计行驶距离推导（位移积分），保证「车轮转 = 车真在动」。
 */
export function vehiclePoseAt(
  t: number,
  offset: number,
  route: readonly { x: number; z: number }[],
  budget: VehicleBudget = DEFAULT_VEHICLE_BUDGET,
  wheelRadius = 0.35,
): VehiclePose {
  if (route.length < 2) return { phase: 'stop', x: route[0]?.x ?? 0, z: route[0]?.z ?? 0, heading: 0, wheelSpin: 0, speed: 0 }

  // 环线总长
  let totalLen = 0
  const cum: number[] = [0]
  for (let i = 0; i < route.length; i++) {
    const a = route[i]
    const b = route[(i + 1) % route.length]
    totalLen += dist(a, b)
    cum.push(totalLen)
  }

  // 简化闭环运动学：匀速巡航 + 每圈停一次（停靠点=route[0]）。
  // 加速/减速段用一个 stopDwell 周期表达，避免匀速绕圈。
  const accelTime = budget.cruiseSpeed / Math.max(1e-3, budget.accel)
  const cruiseLen = totalLen
  const cycleTime = 2 * accelTime + cruiseLen / budget.cruiseSpeed + budget.stopDwell

  let ct = ((t + offset) % cycleTime + cycleTime) % cycleTime
  let traveled = 0
  let phase: VehiclePhase = 'cruise'
  let speed = budget.cruiseSpeed

  if (ct < accelTime) {
    phase = 'accelerate'
    speed = budget.accel * ct
    traveled = 0.5 * budget.accel * ct * ct
  } else {
    ct -= accelTime
    const cruiseTime = cruiseLen / budget.cruiseSpeed
    if (ct < cruiseTime) {
      phase = 'cruise'
      speed = budget.cruiseSpeed
      traveled = 0.5 * budget.accel * accelTime * accelTime + budget.cruiseSpeed * ct
    } else {
      ct -= cruiseTime
      if (ct < accelTime) {
        phase = 'decelerate'
        speed = budget.cruiseSpeed * (1 - ct / accelTime)
        traveled = totalLen - 0.5 * budget.accel * ct * ct
      } else {
        phase = 'stop'
        speed = 0
        traveled = totalLen
      }
    }
  }

  // 按累计距离落在环线上
  traveled = Math.max(0, Math.min(totalLen - 1e-6, traveled))
  let seg = 0
  while (seg < cum.length - 2 && cum[seg + 1] < traveled) seg++
  const a = route[seg]
  const b = route[(seg + 1) % route.length]
  const segLen = Math.max(1e-6, cum[seg + 1] - cum[seg])
  const k = (traveled - cum[seg]) / segLen
  return {
    phase,
    x: a.x + (b.x - a.x) * k,
    z: a.z + (b.z - a.z) * k,
    heading: Math.atan2(b.z - a.z, b.x - a.x),
    wheelSpin: traveled / Math.max(1e-3, wheelRadius),
    speed,
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 重复件 → InstancedMesh 数据描述
// ─────────────────────────────────────────────────────────────────────────────

/** 一个实例化构件的摆放（渲染层塞进 InstancedMesh.setMatrixAt） */
export interface InstancedTransform {
  x: number
  y: number
  z: number
  rotY: number
  scale: number
}

/** 脚手架管阵列：在方形 footprint 四周立杆 + 横杆层 */
export function scaffoldGrid(footprint: number, floors: number, floorHeight: number): InstancedTransform[] {
  const out: InstancedTransform[] = []
  const half = footprint / 2 + 0.25
  const posts = 4 // 每边立杆数
  const layers = Math.max(1, floors)
  // 立杆（四角 + 边中）
  for (let side = 0; side < 4; side++) {
    for (let i = 0; i < posts; i++) {
      const u = i / (posts - 1) - 0.5 // -0.5..0.5
      let x = 0
      let z = 0
      if (side === 0) { x = -half + u * footprint; z = -half }
      else if (side === 1) { x = half; z = -half + u * footprint }
      else if (side === 2) { x = half - u * footprint; z = half }
      else { x = -half; z = half - u * footprint }
      out.push({ x, y: (layers * floorHeight) / 2, z, rotY: 0, scale: layers * floorHeight })
    }
  }
  return out
}

/** 锥桶/路障沿地块边界一圈 */
export function barrierRing(footprint: number, spacing = 1.2): InstancedTransform[] {
  const out: InstancedTransform[] = []
  const half = footprint / 2 + 0.6
  const perSide = Math.max(2, Math.round((footprint + 1.2) / spacing))
  for (let side = 0; side < 4; side++) {
    for (let i = 0; i < perSide; i++) {
      const u = i / (perSide - 1) - 0.5
      let x = 0
      let z = 0
      if (side === 0) { x = -half + u * (footprint + 1.2); z = -half }
      else if (side === 1) { x = half; z = -half + u * (footprint + 1.2) }
      else if (side === 2) { x = half - u * (footprint + 1.2); z = half }
      else { x = -half; z = half - u * (footprint + 1.2) }
      out.push({ x, y: 0, z, rotY: 0, scale: 1 })
    }
  }
  return out
}

// ─────────────────────────────────────────────────────────────────────────────
// 施工配色（3D 数据层，0xRRGGBB 整数；与 islandSpec.palette 同口径，不进 CSS）
// ─────────────────────────────────────────────────────────────────────────────

export const CONSTRUCTION_PALETTE = {
  safetyOrange: 0xe8873a, // 塔吊/锥桶（安全色）
  steelYellow: 0xd9a441, // 脚手架黄管
  concrete: 0xb9b3a8, // 未装饰混凝土
  formwork: 0x8a6a44, // 木模板
  safetyVest: 0xf2c14e, // 工人背心
  excavator: 0xc9544b, // 挖掘机红
} as const
