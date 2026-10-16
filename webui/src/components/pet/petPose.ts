/**
 * 桌宠动画纯函数引擎（v3）— 全部「表情/运动数学」集中于此，可单测、无副作用。
 *
 * 设计动机（engineering-core 铁律 R2）：
 * - 旧版动画数学散落在 Pet3D 的 update* 方法里，WebGL 环境下无法单测。
 * - `computePose` 为纯函数：输入（spec + mood + 时间 + 状态）→ 输出完整位姿，
 *   随机性（眨眼/抽耳时机）由外层计时器算好后传入，保证确定性可测。
 */

import type { PetMood3D } from './petContract'
import type { PetModelSpec } from './petSpec'
import { PET_HOVER_FOCUS, PET_INTERACTION } from './petSpec'

/** 部位点击热区（头 / 背 / 尾） */
export type PetHitRegion = 'head' | 'back' | 'tail'

/** 部位反应输入（elapsed = 点击发生后经过的秒数，驱动衰减冲量） */
export interface PokeInput {
  region: PetHitRegion
  elapsed: number
}

/** computePose 输入（随机状态已由外层解算，本函数确定性） */
export interface PoseInput {
  mood: PetMood3D
  /** 累计秒（动画相位） */
  t: number
  /** 唤醒度 0..1：调制尾频/耳幅/呼吸 */
  arousal: number
  /** -1..1 视线目标 */
  gazeX: number
  gazeY: number
  /** 1=睁眼 … 0=全闭（眨眼计时器算好） */
  blinkScaleY: number
  /** 0=无耳抽动；>0 触发抽动振幅 */
  twitchPhase: number
  /** 跳跃高度（物理已解算） */
  jumpY: number
  /** squash&stretch（1=常态；落地挤压 <1 / 弹起拉伸 >1） */
  squash: number
  isWalking: boolean
  walkPhase: number
  isDancing: boolean
  isSpinning: boolean
  /** happy 蹦跳：true = 心情动作「开心蹦跳」进行中（纯函数输出周期性小跳） */
  isBouncing?: boolean
  /** sleepy 揉眼：true = 心情动作「打盹揉眼」进行中（手臂上抬 + 眯眼加重） */
  isRubbingEyes?: boolean
  /** 部位点击反应（null = 无）；由交互层在点击时置入，随时间自然衰减 */
  poke?: PokeInput | null
  /**
   * 悬停聚焦权重 0..1（缺省 0）：光标停在它身上 → 视线与瞳孔幅度放大（「盯着你看」）。
   * 幅度系数取自 `PET_HOVER_FOCUS`（数据驱动），本函数只做确定性加权。
   */
  focus?: number
}

/** computePose 输出：渲染层仅做「值 → 网格」映射 */
export interface Pose {
  // 身体
  bodyScaleX: number
  bodyScaleY: number
  bodyScaleZ: number
  bodyPosY: number
  rootPosY: number
  rootPosX: number
  rootRotZ: number
  // 头
  headRotX: number
  headRotY: number
  headRotZ: number
  // 耳（含情绪 + 抽动）
  earRotZL: number
  earRotZR: number
  // 眼
  eyeScaleYL: number
  eyeScaleYR: number
  eyeRotZL: number
  eyeRotZR: number
  // 瞳孔（缩放 + 视线偏移）
  pupilScale: number
  pupilOffsetX: number
  pupilOffsetY: number
  // 眉
  browRotZL: number
  browRotZR: number
  browPosY: number
  // 嘴
  mouthShape: 'smile' | 'open' | 'frown' | 'swoon' | 'none'
  mouthOpen: number
  // 腮红
  blushOpacity: number
  // 翅膀
  wingFlap: number
  wingRotZL: number
  wingRotZR: number
  // 尾
  tailWag: number
  tailChainWave: number
  // 光环
  haloOpacity: number
  haloRotZ: number
  // 脚
  footPosYL: number
  footPosYR: number
  // 头顶饰物
  flowerRotZ: number
  flowerPosY: number
  /** happy 蹦跳产生的周期性小跳高度（叠加进 rootPosY / bodyPosY） */
  bounceY: number
  /** sleepy 揉眼的手臂抬升量 0..1（人形左/右臂上抬 + 内收；兽形无臂 = 0） */
  armLift: number
  /** 部位点击：头部点头分量（已折叠进 headRotX，此处仅供观测/测试） */
  pokeHeadRotX: number
  /** 部位点击：背部侧倾分量（已折叠进 rootRotZ，此处仅供观测/测试） */
  pokeLeanRotZ: number
  /** 部位点击：尾部甩动增量（已折叠进 tailWag，此处仅供观测/测试） */
  pokeTailBoost: number
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi)
const mood3dSet: PetMood3D[] = ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad']

/** 情绪调制系数：高频心情 → 更快更大幅度（确定性规则） */
function arousalMod(mood: PetMood3D): number {
  switch (mood) {
    case 'excited': return 2.4
    case 'happy': return 1.8
    case 'working': return 1.3
    case 'sleepy': return 0.35
    case 'sad': return 0.6
    default: return 1
  }
}

/** 纯函数：由 spec + 心情 + 时间 → 完整位姿。不存在随机调用。 */
export function computePose(spec: PetModelSpec, input: PoseInput): Pose {
  const { t, arousal, gazeX, gazeY } = input
  const am = arousalMod(input.mood)
  const ep = spec.emotionPoses[input.mood]
  const mo = spec.motion

  // ── 部位点击反应：指数衰减冲量（头=点头 / 背=侧倾 / 尾=甩尾），确定性纯函数 ──
  const poke = input.poke ?? null
  let pokeHeadRotX = 0
  let pokeLeanRotZ = 0
  let pokeTailBoost = 0
  if (poke && poke.elapsed >= 0) {
    const env = Math.exp(-poke.elapsed * mo.poke.decay)
    const osc = Math.sin(poke.elapsed * mo.poke.freq)
    if (poke.region === 'head') pokeHeadRotX = env * mo.poke.headAmp * (0.5 + 0.5 * osc)
    else if (poke.region === 'tail') pokeTailBoost = env * mo.poke.tailAmp * osc
    else pokeLeanRotZ = env * mo.poke.backAmp * osc
  }

  // ── 心情动作：happy 蹦跳 / sleepy 揉眼（进入对应心情时由渲染层持续喂入 true）──
  const isBouncing = input.isBouncing === true
  const isRubbing = input.isRubbingEyes === true
  const bounceY = isBouncing ? Math.abs(Math.sin(t * mo.bounce.freq)) * mo.bounce.amp : 0
  const armLift = isRubbing
    ? mo.rub.armLiftBase + Math.sin(t * 7) * mo.rub.armLiftAmp
    : 0

  // ── 身体：呼吸 + 漂浮 + 情绪缩放 + squash + 跳跃 + 蹦跳 ──
  const breathe = Math.sin(t * mo.breath.freq) * mo.breath.amp
  const floatY = Math.sin(t * mo.float.freq + 0.5) * mo.float.amp
  const sX = (1 + breathe) * (ep.body.scale ?? 1) * input.squash
  const sY = (1 - breathe * 0.5) * (ep.body.scale ?? 1) * (2 - input.squash)
  const sZ = (1 + breathe * 0.3) * (ep.body.scale ?? 1) * input.squash
  const bodyPosY = 0.08 + floatY + (ep.body.posY ?? 0) + input.jumpY + bounceY

  // ── 头：视线追踪 + 情绪位姿（rotZ 摆动仅 happy/excited 高频）；揉眼时轻低头 ──
  // 悬停聚焦：视线幅度按 focus 放大（光标停在身上 = 它更专心看你）
  const focus = Number.isFinite(input.focus) ? Math.min(1, Math.max(0, input.focus as number)) : 0
  const gazeBoost = 1 + PET_HOVER_FOCUS.gazeGain * focus
  const headRotY = clamp(gazeX * 0.42 * (0.6 + arousal * 0.4) * gazeBoost, -0.5, 0.5)
  const headRotX = clamp(-gazeY * 0.22 * gazeBoost, -0.4, 0.3) + (ep.head.rotX ?? 0) + (isRubbing ? 0.1 : 0) + pokeHeadRotX
  const headShake = (input.mood === 'happy' || input.mood === 'excited') ? Math.sin(t * 3) : 0
  const headRotZ = (ep.head.rotZ ?? 0) * (headShake === 0 ? 1 : Math.abs(headShake) + 0.25) + headShake * 0.15

  // ── 耳：情绪基准 + 抽动（耳幅随唤醒度） ──
  const twitch = input.twitchPhase > 0
    ? Math.sin(t * 22) * mo.earTwitch.amp * (0.5 + arousal * 0.5)
    : 0
  const earRotZL = (ep.leftEar.rotZ ?? 0) + twitch
  const earRotZR = (ep.rightEar.rotZ ?? 0) - twitch

  // ── 眼：眨眼（外部算好）+ 眯眼（情绪）+ 悲伤下垂；揉眼时眯得更狠 ──
  const eyeScale = input.blinkScaleY * (ep.leftEye.scaleY ?? 1) * (isRubbing ? mo.rub.eyeSquint : 1)
  const eyeRotZL = ep.leftEye.rotZ ?? 0
  const eyeRotZR = ep.rightEye.rotZ ?? 0

  // ── 瞳孔：情绪放大 + 视线偏移（悬停聚焦时位移加大、瞳孔略放大） ──
  const pupilScale = ep.pupilScale * (1 + PET_HOVER_FOCUS.pupilScaleGain * focus)
  const pupilBoost = 1 + PET_HOVER_FOCUS.pupilGain * focus
  const pupilOffsetX = gazeX * 0.03 * pupilBoost
  const pupilOffsetY = gazeY * 0.025 * pupilBoost

  // ── 眉 ──
  const browRotZL = ep.leftBrow.rotZ ?? 0
  const browRotZR = ep.rightBrow.rotZ ?? 0
  const browPosY = ep.leftBrow.posY ?? 0.22

  // ── 嘴/腮红 ──
  const mouthShape = ep.mouth.shape
  const mouthBase = ep.mouth.open ?? 0
  const mouthOpen = input.mood === 'excited' ? clamp(mouthBase + Math.sin(t * 10) * 0.2, 0, 1.2) : mouthBase
  const blushOpacity = ep.blush.opacity

  // ── 翅膀：情绪 + 跳舞/跳跃放大；困倦减小 ──
  let wingFreq = mo.wing.freq * (input.mood === 'excited' ? 2.3 : input.isDancing ? 2 : am)
  let wingAmp = mo.wing.amp * (input.mood === 'excited' || input.isDancing ? 1.7 : am < 1 ? am : 1)
  if (input.jumpY > 0) { wingFreq = 20; wingAmp = 0.6 }
  const wingFlap = Math.sin(t * wingFreq) * wingAmp
  const wingRotZL = wingFlap * 0.32
  const wingRotZR = -wingFlap * 0.32

  // ── 尾：频率/幅度随情绪（确定性子集） ──
  const tailFreq = mo.tail.freq * (input.mood === 'excited' ? 2.5 : input.mood === 'happy' ? 2 : am)
  const tailAmp = mo.tail.amp * (input.mood === 'sleepy' ? 0.35 : input.mood === 'sad' ? 0.6 : 1)
  const tailWag = Math.sin(t * tailFreq) * tailAmp + (input.mood === 'sad' ? -0.2 : 0) + pokeTailBoost
  const tailChainWave = Math.sin(t * tailFreq * 0.6) * 0.13

  // ── 光环：working/excited/happy 亮起 ──
  let haloOpacity = 0
  if (input.mood === 'working') haloOpacity = 0.55
  if (input.mood === 'excited' || input.mood === 'happy') haloOpacity = 0.75
  if (input.isDancing) haloOpacity = 0.85
  const haloRotZ = t * mo.haloSpin

  // ── 脚：走路弹跳 / 跳舞 / 回落 ──
  let footPosYL = -0.44
  let footPosYR = -0.44
  if (input.isWalking) {
    footPosYL = -0.44 + Math.max(0, Math.sin(input.walkPhase)) * 0.09
    footPosYR = -0.44 + Math.max(0, Math.sin(input.walkPhase + Math.PI)) * 0.09
  } else if (input.isDancing) {
    footPosYL = -0.44 + Math.max(0, Math.sin(t * 10)) * 0.07
    footPosYR = -0.44 + Math.max(0, Math.sin(t * 10 + Math.PI)) * 0.07
  }

  // ── 头顶饰物 ──
  const flowerRotZ = Math.sin(t * 2.5) * 0.18
  const flowerPosY = Math.sin(t * 2) * 0.012

  // ── 跳舞：左右摇摆 + 弹跳（跳跃由外层物理处理） ──
  let rootPosX = 0
  let rootRotZ = pokeLeanRotZ
  if (input.isDancing) {
    const s = Math.sin(t * 8)
    rootPosX = s * 0.16
    rootRotZ += s * 0.13
  }

  return {
    bodyScaleX: sX,
    bodyScaleY: sY,
    bodyScaleZ: sZ,
    bodyPosY,
    rootPosY: input.jumpY + bounceY,
    rootPosX,
    rootRotZ,
    headRotX,
    headRotY,
    headRotZ,
    earRotZL,
    earRotZR,
    eyeScaleYL: eyeScale,
    eyeScaleYR: eyeScale,
    eyeRotZL,
    eyeRotZR,
    pupilScale,
    pupilOffsetX,
    pupilOffsetY,
    browRotZL,
    browRotZR,
    browPosY,
    mouthShape,
    mouthOpen,
    blushOpacity,
    wingFlap,
    wingRotZL,
    wingRotZR,
    tailWag,
    tailChainWave,
    haloOpacity,
    haloRotZ,
    footPosYL,
    footPosYR,
    flowerRotZ,
    flowerPosY,
    bounceY,
    armLift,
    pokeHeadRotX,
    pokeLeanRotZ,
    pokeTailBoost,
  }
}

/** 校验输入心情是否在 spec 覆盖内（防御：未知心情抛错而非静默） */
export function assertMoodSupported(spec: PetModelSpec, mood: PetMood3D): void {
  if (!mood3dSet.includes(mood) || !spec.emotionPoses[mood]) {
    throw new Error(`petPose: 心情 '${mood}' 无 Spec 表情定义（请在 petSpec.emotionPoses 补齐）`)
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// 交互纯函数（部位热区 / 手势判别 / 惯性滑行 / 确定性伪随机）
// 全部无副作用、可单测；渲染层只做「值 → 网格」映射，不在此外再写数学。
// ─────────────────────────────────────────────────────────────────────────────

/**
 * 部位热区：把宠物体框内的纵向归一化位置（0=顶，1=底）映射到头/背/尾三段。
 * 纯函数、无随机 —— 点击头的反应与点击尾不同。
 */
export function regionFromPoint(ny: number): PetHitRegion {
  const { headBelow, tailAbove } = PET_INTERACTION.region
  const y = clamp(ny, 0, 1)
  if (y < headBelow) return 'head'
  if (y > tailAbove) return 'tail'
  return 'back'
}

/** 手势判别结果：tap=轻点（触发部位反应）/ drag=拖动（惯性移动） */
export type PetGesture = 'tap' | 'drag'

/**
 * 手势判别（纯函数）：位移(px) 与按压时长(ms) 同时不超阈值才算轻点，
 * 否则视为拖动 —— 用于区分「轻点部位」与「拖着桌宠走」。
 */
export function classifyGesture(dx: number, dy: number, dtMs: number): PetGesture {
  const dist = Math.hypot(dx, dy)
  return dist <= PET_INTERACTION.tapMaxMove && dtMs <= PET_INTERACTION.tapMaxMs ? 'tap' : 'drag'
}

/** 惯性体（左上角坐标 + 速度 px/s） */
export interface InertiaBody {
  x: number
  y: number
  vx: number
  vy: number
}

/** 惯性边界（可视区内可停留的合法范围） */
export interface InertiaBounds {
  minX: number
  minY: number
  maxX: number
  maxY: number
}

/** 惯性参数（与 PET_INTERACTION.inertia 同构，便于测试注入） */
export interface InertiaConfig {
  friction: number
  minSpeed: number
  restitution: number
  maxDt: number
}

/** 惯性单步结果 */
export interface InertiaStepResult {
  body: InertiaBody
  /** 速度已低于停止阈值 → 可以落定（贴边）并持久化位置 */
  resting: boolean
}

/**
 * 纯函数：惯性滑行一步（指数摩擦衰减 + 触边贴边并轻微回弹）。
 * 松手后桌宠沿拖拽方向滑行，减速到阈值以下就停住 —— 确定性、可单测。
 */
export function stepInertia(
  body: InertiaBody,
  dt: number,
  bounds: InertiaBounds,
  cfg: InertiaConfig = PET_INTERACTION.inertia,
): InertiaStepResult {
  const d = clamp(dt, 0, cfg.maxDt)
  const decay = Math.exp(-cfg.friction * d)
  let vx = body.vx * decay
  let vy = body.vy * decay
  let x = body.x + vx * d
  let y = body.y + vy * d
  // 触边：贴边 + 该轴速度按 restitution 反向衰减（轻微回弹后趋于静止）
  if (x < bounds.minX) { x = bounds.minX; vx = -vx * cfg.restitution }
  else if (x > bounds.maxX) { x = bounds.maxX; vx = -vx * cfg.restitution }
  if (y < bounds.minY) { y = bounds.minY; vy = -vy * cfg.restitution }
  else if (y > bounds.maxY) { y = bounds.maxY; vy = -vy * cfg.restitution }
  const resting = Math.hypot(vx, vy) < cfg.minSpeed
  return {
    body: { x, y, vx: resting ? 0 : vx, vy: resting ? 0 : vy },
    resting,
  }
}

/**
 * 确定性伪随机（mulberry32）：同 seed → 同序列，落在 [0,1)。
 * 渲染层的眨眼时机 / 双眨眼 / 粒子一律由它驱动 —— 禁止再出现 Math.random，
 * 保证「随机小动作」在测试中可复现。
 */
export function createSeededRandom(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}