/**
 * 岛屿模拟层（纯函数）—— 全部动画数学收敛于此，渲染器只做「数值 → 网格」映射。
 *
 * - 确定性：同一 (spec, world, t) 永远得到同一帧；随机性只来自 hash（无 Math.random）。
 * - 无副作用：不读时钟、不碰 DOM、不 import three —— 因此可在 node 环境完整单测。
 * - 昼夜光照（《我们的岛》式夕阳/夜灯）由 skyAt 统一给出：太阳方向、色温、天空渐变、
 *   雾色、月光与灯塔光束强度，渲染器只是把这些数写进灯光与材质。
 */

import { ISLAND_KIND_META } from './islandContract'
import type { IslandModelSpec } from './islandSpec'
import type { IslandWorld } from './islandWorld'

export type IslandPhase = 'dawn' | 'day' | 'dusk' | 'night'

export interface IslandSkyState {
  phase: IslandPhase
  /** 一天中的归一化进度 0..1（0 = 日出） */
  dayProgress: number
  /** 白昼强度 0..1 */
  dayFactor: number
  /** 夜晚强度 0..1（灯笼/月光/灯塔光束） */
  nightFactor: number
  /** 太阳高度角（弧度，负值 = 地平线下） */
  sunAngle: number
  sunDirection: { x: number; y: number; z: number }
  sunColor: number
  sunIntensity: number
  ambientColor: number
  ambientIntensity: number
  skyTop: number
  skyBottom: number
  fogColor: number
  /** 灯塔光束强度 0..1 */
  beamIntensity: number
}

export interface IslandEntityVisual {
  id: string
  /** 灯笼/窗口亮度 0..1 */
  glow: number
  /** 呼吸脉动 0..1 */
  pulse: number
  /** 是否点亮灯笼（低质量档可据此关灯） */
  lantern: boolean
  /** 风摆相位（−1..1） */
  sway: number
  /** 轻微上下浮动（米） */
  bob: number
}

export interface IslandTraffic {
  linkId: string
  /** 货物沿道路的行进参数 0..1（−1 = 该路无流动） */
  phase: number
  active: boolean
}

export interface IslandFrame {
  sky: IslandSkyState
  entities: IslandEntityVisual[]
  traffic: IslandTraffic[]
}

/* ── 基础数学（导出以便复用与单测）── */

export function clamp(value: number, min: number, max: number): number {
  return value < min ? min : value > max ? max : value
}

export function clamp01(value: number): number {
  return clamp(value, 0, 1)
}

export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

export function fract(value: number): number {
  return value - Math.floor(value)
}

/** 平滑阶跃（用于昼夜过渡：宽窗口 = 漫长的黄昏，避免日落瞬间突变） */
export function smoothstep(edge0: number, edge1: number, x: number): number {
  const span = edge1 - edge0
  if (span === 0) return x < edge0 ? 0 : 1
  const t = clamp01((x - edge0) / span)
  return t * t * (3 - 2 * t)
}

/** 颜色通道线性插值（整数 0xRRGGBB） */
export function mixColor(a: number, b: number, t: number): number {
  const k = clamp01(t)
  const ar = (a >> 16) & 0xff
  const ag = (a >> 8) & 0xff
  const ab = a & 0xff
  const br = (b >> 16) & 0xff
  const bg = (b >> 8) & 0xff
  const bb = b & 0xff
  const r = Math.round(lerp(ar, br, k))
  const g = Math.round(lerp(ag, bg, k))
  const bl = Math.round(lerp(ab, bb, k))
  return (r << 16) | (g << 8) | bl
}

/** 明暗缩放（factor < 1 变暗），保持色相 */
export function shade(color: number, factor: number): number {
  return mixColor(0x000000, color, clamp01(factor))
}

/** 32 位 FNV-1a 字符串哈希（稳定 → 同 id 的建筑永远同一相位） */
export function hashString(input: string): number {
  let hash = 0x811c9dc5
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193) >>> 0
  }
  return hash >>> 0
}

/** 整数 → [0,1) 确定性散列（地形抖动等，替代 Math.random） */
export function hashUnit(seed: number): number {
  let x = (seed | 0) + 0x9e3779b9
  x = Math.imul(x ^ (x >>> 16), 0x21f0aaad) >>> 0
  x = Math.imul(x ^ (x >>> 15), 0x735a2d97) >>> 0
  return ((x ^ (x >>> 15)) >>> 0) / 4294967296
}

/* ── 昼夜与光照 ── */

interface ColorStop {
  at: number
  color: number
}

/** 在循环色带上取色（stops 必须按 at 升序，覆盖 0..1） */
function cycleColor(stops: readonly ColorStop[], progress: number): number {
  const p = fract(progress)
  for (let i = 0; i < stops.length - 1; i++) {
    const from = stops[i]
    const to = stops[i + 1]
    if (p >= from.at && p <= to.at) {
      const span = to.at - from.at
      return mixColor(from.color, to.color, span <= 0 ? 0 : (p - from.at) / span)
    }
  }
  return stops[stops.length - 1].color
}

/** 一天中的光照状态（含夕阳色温与夜空渐变） */
export function skyAt(spec: IslandModelSpec, t: number): IslandSkyState {
  const dayProgress = fract(t / spec.lighting.dayLength)
  const sunAngle = dayProgress * Math.PI * 2
  const elevation = Math.sin(sunAngle)
  // 宽窗口 smoothstep：日落/日出有数秒渐变（避免灯光硬跳）
  const dayFactor = smoothstep(-0.62, 0.48, elevation)
  const nightFactor = 1 - smoothstep(-0.5, 0.5, elevation)

  const phase: IslandPhase =
    elevation <= -0.05 ? 'night' : elevation >= 0.35 ? 'day' : dayProgress < 0.25 ? 'dawn' : 'dusk'

  // 太阳方向：沿 XZ 斜射，Y 为高度（夜晚落到地平线下由 moonColor 接管）
  const dirLen = Math.hypot(Math.cos(sunAngle) * 0.82, elevation + 0.06, 0.42) || 1
  const sunDirection = {
    x: (Math.cos(sunAngle) * 0.82) / dirLen,
    y: (elevation + 0.06) / dirLen,
    z: 0.42 / dirLen,
  }

  const L = spec.lighting
  const P = spec.palette
  const skyTopDay = mixColor(P.water, L.dayColor, 0.5)
  const skyBottomDay = mixColor(P.cloud, L.dayColor, 0.35)
  const skyTopDusk = shade(L.duskColor, 0.55)
  const skyBottomDusk = L.duskColor
  const skyTopNight = shade(L.nightColor, 0.2)
  const skyBottomNight = shade(L.nightColor, 0.34)

  // 色带键位：0=日出前（保留暮色）→ 白天 → 日落 → 夜
  const stops = (day: number, dusk: number, night: number): ColorStop[] => [
    { at: 0, color: dusk },
    { at: 0.07, color: day },
    { at: 0.43, color: day },
    { at: 0.5, color: dusk },
    { at: 0.58, color: night },
    { at: 0.93, color: night },
    { at: 1, color: dusk },
  ]

  const sunColorTrack = stops(L.dayColor, L.duskColor, L.nightColor)
  // 白昼→夜晚平滑交接：光色由「夕阳色带」渐变到月光，避免硬切跳色
  const sunColor = mixColor(
    L.moonColor,
    cycleColor(sunColorTrack, dayProgress),
    smoothstep(0.02, 0.3, dayFactor),
  )
  const skyTop = cycleColor(stops(skyTopDay, skyTopDusk, skyTopNight), dayProgress)
  const skyBottom = cycleColor(stops(skyBottomDay, skyBottomDusk, skyBottomNight), dayProgress)

  return {
    phase,
    dayProgress,
    dayFactor,
    nightFactor,
    sunAngle,
    sunDirection,
    sunColor,
    sunIntensity: 0.22 + 2.1 * dayFactor,
    ambientColor: skyBottom,
    ambientIntensity: lerp(L.ambientNight, L.ambientDay, dayFactor),
    skyTop,
    skyBottom,
    fogColor: mixColor(skyBottom, P.cloud, 0.22 * dayFactor),
    beamIntensity: 0.15 + 0.85 * nightFactor,
  }
}

/* ── 水面与实体动画 ── */

/** 相对亮度（0..255 加权）——用于「浅滩必须比远海亮」这类配色不变量校验 */
export function luminance(color: number): number {
  return ((color >> 16) & 0xff) * 0.2126 + ((color >> 8) & 0xff) * 0.7152 + (color & 0xff) * 0.0722
}

/**
 * 水体径向色阶（唯美海面的关键）：**近岸浅滩 → 主体海色 → 远海深水**。
 *
 * `t` = 离岸归一化距离：0 = 岸线（水面内圈），1 = 水面外缘。
 *
 * 为什么要径向色阶：此前水面是**单色 + 雾**——近岸与远海只有雾造成的亮度差，
 * 缺少《我们的岛》里那种「浅滩透亮青绿、远海沉静靛蓝」的层次。而运行时实测
 * 水面占画面约 85%（把 palette.water 改红即可看清），所以水的层次几乎决定了
 * 整个场景的唯美程度。
 *
 * 浅滩带刻意做窄（前 30%）：过宽会让整片水发白、失去海的重量。
 */
export function waterColorAt(spec: IslandModelSpec, t: number): number {
  const P = spec.palette
  const u = clamp01(t)
  if (u <= 0.3) return mixColor(P.waterShallow, P.water, smoothstep(0, 0.3, u))
  return mixColor(P.water, P.waterDeep, smoothstep(0.3, 1, u))
}

/**
 * 水体昼夜色温染色（乘在顶点色之上）。
 * 白天必须**恰好是中性白**（0xffffff）：否则会污染 waterColorAt 的径向色阶。
 * 入夜整体压暗偏靛蓝，但不得压到发黑（海面在夜里仍要看得见层次）。
 */
export function waterTintAt(spec: IslandModelSpec, nightFactor: number): number {
  return mixColor(0xffffff, spec.palette.waterDeep, clamp01(nightFactor) * 0.55)
}

/**
 * 逆光补光方向：太阳方位的**水平对侧** + 固定中等仰角。
 *
 * 为什么需要：岛缘侧壁/悬崖面的法线朝外朝下，N·L ≈ 0，只有半球光暗侧照到 →
 * 会在岛周围压出一圈近黑的「描边」。补光从对侧斜打正好落在这些面上。
 * 固定仰角（不跟随太阳高度）保证日出/日落时补光不会跟着沉到地平线下而失效。
 */
export function fillDirection(sky: IslandSkyState): { x: number; y: number; z: number } {
  const d = sky.sunDirection
  const len = Math.hypot(d.x, d.z) || 1
  return { x: -d.x / len, y: 0.5, z: -d.z / len }
}

/** 水面波高（CPU 顶点位移用；两向正弦叠加，确定性） */
export function waterWaveAt(spec: IslandModelSpec, x: number, z: number, t: number): number {
  const W = spec.water
  const k = (Math.PI * 2) / W.waveLength
  const a = Math.sin(x * k + t * W.waveSpeed) * W.waveHeight * 0.62
  const b = Math.cos(z * k * 0.8 - t * W.waveSpeed * 0.7) * W.waveHeight * 0.38
  return clamp(a + b, -W.waveHeight, W.waveHeight)
}

/** 状态 → 基础亮度（idle 也留一点微光，让岛屿不至于死黑） */
const STATUS_GLOW: Record<string, number> = {
  idle: 0.1,
  queued: 0.3,
  active: 1,
  success: 0.42,
  error: 0.75,
  waiting: 0.5,
}

/** 单实体视觉状态（灯笼/呼吸/风摆） */
export function entityVisual(
  spec: IslandModelSpec,
  entity: { id: string; kind: keyof typeof ISLAND_KIND_META; status: string },
  sky: IslandSkyState,
  t: number,
): IslandEntityVisual {
  const seed = hashString(entity.id)
  const phase = ((seed % 1000) / 1000) * Math.PI * 2
  const pulse = 0.5 + 0.5 * Math.sin(t * spec.motion.pulseHz * Math.PI * 2 + phase)
  const base = STATUS_GLOW[entity.status] ?? 0.1

  let glow = base
  if (entity.status === 'active') glow = 0.6 + 0.4 * pulse
  else if (entity.status === 'error') glow = 0.45 + 0.55 * (0.5 + 0.5 * Math.sin(t * Math.PI * 4 + phase))
  else if (entity.status === 'waiting') glow = 0.4 + 0.2 * pulse
  // 夜晚灯笼更亮（低对比度时靠灯光补足可读性）
  glow = clamp01(glow * (0.72 + 0.38 * sky.nightFactor))

  const meta = ISLAND_KIND_META[entity.kind]
  return {
    id: entity.id,
    glow,
    pulse,
    lantern: meta.glow && glow > 0.18,
    sway: Math.sin(t * 0.6 + phase) * spec.motion.windSway,
    bob: Math.sin(t * 1.1 + phase * 1.7) * 0.022,
  }
}

/**
 * 一帧完整模拟结果（渲染器逐帧消费）。
 * `dayTime` 允许与动画时间解耦：用户手动拖动昼夜时，灯光按 dayTime 走，
 * 而水波/呼吸/货物流仍按真实时间推进。
 */
export function computeIslandFrame(
  spec: IslandModelSpec,
  world: IslandWorld,
  t: number,
  dayTime: number = t,
): IslandFrame {
  const sky = skyAt(spec, dayTime)
  const entities = world.entities.map((entity) => entityVisual(spec, entity as never, sky, t))
  const traffic: IslandTraffic[] = world.links.map((link) => {
    const offset = (hashString(link.id) % 1000) / 1000
    return {
      linkId: link.id,
      active: link.active,
      phase: link.active ? fract(t * spec.motion.cargoSpeed * 0.4 + offset) : -1,
    }
  })
  return { sky, entities, traffic }
}

/* ── 轨道相机（交互数学，供渲染器与 2D 降级共用）── */

export interface OrbitCamera {
  yaw: number
  pitch: number
  distance: number
  targetX: number
  targetY: number
  targetZ: number
}

export interface IslandCameraLimits {
  minDistance: number
  maxDistance: number
  minPitch: number
  maxPitch: number
  maxTargetRadius: number
}

/** 相机约束由地形半径推导（岛屿越大，允许离得越远） */
export function islandCameraLimits(spec: IslandModelSpec): IslandCameraLimits {
  const r = spec.terrain.radius
  return {
    minDistance: r * 0.55,
    maxDistance: r * 4.4,
    minPitch: 0.12,
    maxPitch: 1.34,
    maxTargetRadius: r * 0.7,
  }
}

export function createIslandCamera(spec: IslandModelSpec): OrbitCamera {
  return {
    yaw: spec.layout.quayAngle + Math.PI * 0.62,
    pitch: 0.63,
    distance: spec.terrain.radius * 2.6,
    targetX: 0,
    targetY: 0.6,
    targetZ: 0,
  }
}

export function cameraPosition(cam: OrbitCamera): { x: number; y: number; z: number } {
  const horizontal = cam.distance * Math.cos(cam.pitch)
  return {
    x: cam.targetX + horizontal * Math.sin(cam.yaw),
    y: cam.targetY + cam.distance * Math.sin(cam.pitch),
    z: cam.targetZ + horizontal * Math.cos(cam.yaw),
  }
}

function clampTarget(cam: OrbitCamera, limits: IslandCameraLimits): OrbitCamera {
  const radius = Math.hypot(cam.targetX, cam.targetZ)
  if (radius <= limits.maxTargetRadius || radius === 0) return cam
  const k = limits.maxTargetRadius / radius
  return { ...cam, targetX: cam.targetX * k, targetZ: cam.targetZ * k }
}

/** 拖拽旋转（dx/dy 为像素位移，sensitivity 由渲染器按画布高度折算） */
export function orbitCamera(cam: OrbitCamera, dx: number, dy: number, spec: IslandModelSpec): OrbitCamera {
  const limits = islandCameraLimits(spec)
  return {
    ...cam,
    yaw: cam.yaw + dx,
    pitch: clamp(cam.pitch + dy, limits.minPitch, limits.maxPitch),
  }
}

export function zoomCamera(cam: OrbitCamera, delta: number, spec: IslandModelSpec): OrbitCamera {
  const limits = islandCameraLimits(spec)
  return { ...cam, distance: clamp(cam.distance * Math.exp(delta), limits.minDistance, limits.maxDistance) }
}

/** 平移（沿相机右向/前向的平面投影移动目标点，并限制在岛域内） */
export function panCamera(cam: OrbitCamera, dx: number, dy: number, spec: IslandModelSpec): OrbitCamera {
  const rightX = Math.cos(cam.yaw)
  const rightZ = -Math.sin(cam.yaw)
  const fwdX = -Math.sin(cam.yaw)
  const fwdZ = -Math.cos(cam.yaw)
  const moved: OrbitCamera = {
    ...cam,
    targetX: cam.targetX + rightX * dx + fwdX * dy,
    targetZ: cam.targetZ + rightZ * dx + fwdZ * dy,
  }
  return clampTarget(moved, islandCameraLimits(spec))
}

/** 键盘导航步长（集中一处，便于调参与单测断言） */
export const ISLAND_KEYBOARD = {
  /** 方向键每次的偏航增量（弧度） */
  orbitYaw: 0.09,
  /** 方向键每次的俯仰增量（弧度） */
  orbitPitch: 0.05,
  /** 缩放键每次的增量（传给 zoomCamera：正 = 拉远） */
  zoomStep: 0.22,
  /** Shift+方向键每次的平移步长（世界单位） */
  panStep: 0.7,
} as const

/** 键盘 → 相机命令（`null` = 该键不归相机管，交给上层，如 Esc 关面板） */
export type IslandCameraCommand =
  | { kind: 'orbit'; yaw: number; pitch: number }
  | { kind: 'zoom'; delta: number }
  | { kind: 'pan'; dx: number; dy: number }
  | { kind: 'reset' }

/**
 * 键盘 → 相机命令的**纯映射**。
 *
 * 为什么要有键盘导航：岛屿此前**只有指针交互**（拖拽/滚轮/右拖），键盘用户与
 * 想要精细微调的用户完全无法操作 —— 这对「职业用户」是一处硬缺口。
 * 键位沿用 3D 工具惯例：方向键绕行、Shift+方向键平移、`+`/`-` 缩放、`0` 复位。
 * 纯函数化后，键位与步长可以脱离 WebGL 单测。
 */
export function keyboardCameraCommand(key: string, shiftKey = false): IslandCameraCommand | null {
  const K = ISLAND_KEYBOARD
  switch (key) {
    case 'ArrowLeft':
      return shiftKey ? { kind: 'pan', dx: -K.panStep, dy: 0 } : { kind: 'orbit', yaw: -K.orbitYaw, pitch: 0 }
    case 'ArrowRight':
      return shiftKey ? { kind: 'pan', dx: K.panStep, dy: 0 } : { kind: 'orbit', yaw: K.orbitYaw, pitch: 0 }
    // ↑/↓ 抬高/压低视角（俯仰受 islandCameraLimits 夹紧，不会翻面）
    case 'ArrowUp':
      return shiftKey ? { kind: 'pan', dx: 0, dy: K.panStep } : { kind: 'orbit', yaw: 0, pitch: K.orbitPitch }
    case 'ArrowDown':
      return shiftKey ? { kind: 'pan', dx: 0, dy: -K.panStep } : { kind: 'orbit', yaw: 0, pitch: -K.orbitPitch }
    case '+':
    case '=':
      return { kind: 'zoom', delta: -K.zoomStep }
    case '-':
    case '_':
      return { kind: 'zoom', delta: K.zoomStep }
    case '0':
      return { kind: 'reset' }
    default:
      return null
  }
}

/** 指数阻尼（帧率无关：rate 为每秒收敛速度） */
export function dampCamera(current: OrbitCamera, target: OrbitCamera, dt: number, rate = 9): OrbitCamera {
  const k = 1 - Math.exp(-rate * Math.max(0, dt))
  return {
    yaw: lerp(current.yaw, target.yaw, k),
    pitch: lerp(current.pitch, target.pitch, k),
    distance: lerp(current.distance, target.distance, k),
    targetX: lerp(current.targetX, target.targetX, k),
    targetY: lerp(current.targetY, target.targetY, k),
    targetZ: lerp(current.targetZ, target.targetZ, k),
  }
}

/** 聚焦某点：视点移过去，距离收敛到该建筑的近景 */
export function focusCamera(cam: OrbitCamera, point: { x: number; z: number }, spec: IslandModelSpec): OrbitCamera {
  const limits = islandCameraLimits(spec)
  return clampTarget(
    {
      ...cam,
      targetX: point.x,
      targetZ: point.z,
      targetY: 0.7,
      distance: clamp(spec.terrain.radius * 1.05, limits.minDistance, limits.maxDistance),
    },
    limits,
  )
}