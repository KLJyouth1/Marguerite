/**
 * 星夜画布的纯数学层（可在 Node 里单测，不依赖 canvas / DOM）。
 *
 * 设计目标：
 * - 同一主题每次进入，星位/花瓣序必须**一致**（种子随机），否则“回来看还是那片星空”不成立；
 * - 所有函数都是无副作用的纯函数，方便做视觉回归与性能预算；
 * - 参数只用归一化坐标（0..1），渲染层再乘画布尺寸，便于适配任意窗口。
 */

export interface Star {
  /** 归一化坐标 */
  x: number
  y: number
  /** 半径（逻辑像素，渲染层再乘 DPR 由 canvas 处理） */
  r: number
  /** 闪烁相位（弧度） */
  phase: number
  /** 闪烁速度（rad/s） */
  speed: number
  /** 0=冷白 1=月蓝 2=海蓝宝石 */
  tone: 0 | 1 | 2
}

export interface Petal {
  x: number
  y: number
  /** 下落速度（归一化高度/秒） */
  vy: number
  /** 水平摆幅（归一化宽度） */
  sway: number
  /** 摆动频率（rad/s） */
  swaySpeed: number
  phase: number
  /** 花瓣半径（逻辑像素） */
  size: number
  /** 旋转速度（rad/s） */
  spin: number
  rotation: number
  /** 透明度上限 */
  alpha: number
}

/** 确定性随机（mulberry32）—— 同一 seed 必然产出同一天空。 */
export function seededRandom(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export function createStars(count: number, rand: () => number): Star[] {
  const n = Math.max(0, Math.floor(count))
  const stars: Star[] = []
  for (let i = 0; i < n; i += 1) {
    const roll = rand()
    stars.push({
      x: rand(),
      y: rand(),
      r: 0.45 + rand() * 1.25,
      phase: rand() * Math.PI * 2,
      speed: 0.5 + rand() * 2.2,
      tone: roll > 0.92 ? 2 : roll > 0.72 ? 1 : 0,
    })
  }
  return stars
}

/** 闪烁透明度：0.25..0.95 的正弦呼吸（下限保证“星星不会消失”）。 */
export function starAlpha(t: number, star: Star): number {
  const wave = 0.5 + 0.5 * Math.sin(t * star.speed + star.phase)
  return 0.25 + wave * 0.7
}

export function createPetals(count: number, rand: () => number): Petal[] {
  const n = Math.max(0, Math.floor(count))
  const petals: Petal[] = []
  for (let i = 0; i < n; i += 1) {
    petals.push({
      x: rand(),
      y: rand(),
      vy: 0.012 + rand() * 0.026,
      sway: 0.02 + rand() * 0.05,
      swaySpeed: 0.35 + rand() * 0.8,
      phase: rand() * Math.PI * 2,
      size: 2.6 + rand() * 3.4,
      spin: (rand() - 0.5) * 1.2,
      rotation: rand() * Math.PI * 2,
      alpha: 0.35 + rand() * 0.45,
    })
  }
  return petals
}

/**
 * 推进一片花瓣。返回新对象（不原地修改），便于在渲染层保留上一帧做插值。
 * 落出下边界后从顶部随机重生（x 重新撒点），保持“永远在下花瓣”。
 */
export function stepPetal(petal: Petal, dt: number, rand: () => number): Petal {
  const d = Math.max(0, Math.min(0.1, dt))
  const nextY = petal.y + petal.vy * d
  const nextPhase = petal.phase + petal.swaySpeed * d
  if (nextY > 1.05) {
    return {
      ...petal,
      x: rand(),
      y: -0.05,
      phase: rand() * Math.PI * 2,
    }
  }
  return {
    ...petal,
    y: nextY,
    phase: nextPhase,
    rotation: petal.rotation + petal.spin * d,
  }
}

/** 花瓣的水平位置（在基准 x 上叠加正弦摆幅）。 */
export function petalX(petal: Petal): number {
  return petal.x + Math.sin(petal.phase) * petal.sway
}

/** 下一次流星的时间间隔（毫秒）：9–19 秒，避免过密。 */
export function nextShootingStarDelay(rand: () => number): number {
  return 9000 + Math.floor(rand() * 10000)
}

/** 指针视差：depth 越大位移越大；输入为 -1..1，输出为逻辑像素偏移。 */
export function parallaxOffset(px: number, py: number, depth: number, maxPx = 10): { x: number; y: number } {
  const clamp = (v: number) => Math.max(-1, Math.min(1, v))
  return {
    x: clamp(px) * depth * maxPx,
    y: clamp(py) * depth * maxPx * 0.6,
  }
}
