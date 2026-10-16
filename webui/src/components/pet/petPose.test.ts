import { describe, expect, it } from 'vitest'
import {
  assertMoodSupported,
  classifyGesture,
  computePose,
  createSeededRandom,
  regionFromPoint,
  stepInertia,
  type PoseInput,
} from './petPose'
import { DEFAULT_PET_SPEC, PET_HOVER_FOCUS, PET_INTERACTION } from './petSpec'
import type { PetMood3D } from './petContract'

function base(): PoseInput {
  return {
    mood: 'normal',
    t: 0,
    arousal: 0.5,
    gazeX: 0,
    gazeY: 0,
    blinkScaleY: 1,
    twitchPhase: 0,
    jumpY: 0,
    squash: 1,
    isWalking: false,
    walkPhase: 0,
    isDancing: false,
    isSpinning: false,
  }
}

describe('petPose —— 动画纯函数引擎', () => {
  it('确定性：同输入两次输出一致（无随机副作用）', () => {
    const a = computePose(DEFAULT_PET_SPEC, base())
    const b = computePose(DEFAULT_PET_SPEC, base())
    expect(a).toEqual(b)
  })

  it('全部心情可计算且数值有限（未知心情必须抛错而非静默）', () => {
    for (const m of ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad'] as PetMood3D[]) {
      expect(() => assertMoodSupported(DEFAULT_PET_SPEC, m)).not.toThrow()
      const pose = computePose(DEFAULT_PET_SPEC, { ...base(), mood: m })
      for (const [k, v] of Object.entries(pose)) {
        if (typeof v === 'number') {
          expect(Number.isFinite(v), `${m}.${k}`).toBe(true)
        }
      }
      expect(['smile', 'open', 'frown', 'swoon', 'none']).toContain(pose.mouthShape)
    }
    expect(() => assertMoodSupported(DEFAULT_PET_SPEC, 'oops' as PetMood3D)).toThrow()
  })

  it('sleepy：眯眼 + 低唤醒尾摆；excited：张嘴 + 高尾频 + 亮光环', () => {
    const sleepy = computePose(DEFAULT_PET_SPEC, { ...base(), mood: 'sleepy', arousal: 0.12 })
    expect(sleepy.eyeScaleYL).toBeLessThan(0.2)
    expect(Math.abs(sleepy.tailWag)).toBeLessThanOrEqual(DEFAULT_PET_SPEC.motion.tail.amp * 0.35 + 1e-9)

    const excited = computePose(DEFAULT_PET_SPEC, { ...base(), mood: 'excited', arousal: 0.95 })
    expect(excited.mouthShape).toBe('open')
    expect(excited.haloOpacity).toBeGreaterThan(0.5)
  })

  it('眨眼：完全闭眼时眼缩放归零；睁眼回到情绪基值', () => {
    const closed = computePose(DEFAULT_PET_SPEC, { ...base(), blinkScaleY: 0 })
    expect(closed.eyeScaleYL).toBeCloseTo(0)
    const open = computePose(DEFAULT_PET_SPEC, { ...base(), blinkScaleY: 1 })
    expect(open.eyeScaleYL).toBeCloseTo(1)
  })

  it('悬停聚焦：视线/瞳孔幅度放大，focus=0 与缺省逐字节等价（向后兼容）', () => {
    const g = { ...base(), gazeX: 0.6, gazeY: -0.4 }
    const off = computePose(DEFAULT_PET_SPEC, g)
    const omitted = computePose(DEFAULT_PET_SPEC, { ...g, focus: undefined })
    expect(omitted).toEqual(off)

    const on = computePose(DEFAULT_PET_SPEC, { ...g, focus: 1 })
    // 注视更用力：头转得更多、瞳孔位移更大、瞳孔略放大
    expect(Math.abs(on.headRotY)).toBeGreaterThan(Math.abs(off.headRotY))
    expect(Math.abs(on.pupilOffsetX)).toBeGreaterThan(Math.abs(off.pupilOffsetX))
    expect(Math.abs(on.pupilOffsetY)).toBeGreaterThan(Math.abs(off.pupilOffsetY))
    expect(on.pupilScale).toBeGreaterThan(off.pupilScale)
    // 增益取自 PET_HOVER_FOCUS（不是随手写的魔数）
    const boost = 1 + PET_HOVER_FOCUS.gazeGain
    expect(on.headRotY).toBeCloseTo(off.headRotY * boost, 10)
    expect(on.pupilOffsetX).toBeCloseTo(off.pupilOffsetX * (1 + PET_HOVER_FOCUS.pupilGain), 10)
    expect(on.pupilScale).toBeCloseTo(off.pupilScale * (1 + PET_HOVER_FOCUS.pupilScaleGain), 10)
  })

  it('悬停聚焦：越界/非法输入被夹住，不放大成「甩头」', () => {
    // gaze 极大 + focus 极大：headRotY 仍受既有 ±0.5 夹紧（聚焦不能突破骨骼极限）
    const wild = computePose(DEFAULT_PET_SPEC, { ...base(), gazeX: 9, gazeY: 9, focus: 99 })
    expect(Math.abs(wild.headRotY)).toBeLessThanOrEqual(0.5)
    // NaN 当 0：与关闭聚焦等价（异常输入不得污染姿态）
    const nan = computePose(DEFAULT_PET_SPEC, { ...base(), gazeX: 0.6, focus: Number.NaN })
    const off = computePose(DEFAULT_PET_SPEC, { ...base(), gazeX: 0.6 })
    expect(nan).toEqual(off)
  })

  it('跳舞：左右摇摆（rootPosX 非零）与跳跃高度透传', () => {
    const dance = computePose(DEFAULT_PET_SPEC, { ...base(), isDancing: true, t: Math.PI / 16, jumpY: 0.1 })
    expect(dance.rootPosX).not.toBeCloseTo(0)
    expect(dance.rootPosY).toBeCloseTo(0.1)
  })

  it('视线追踪：gaze 偏移瞳孔且头随动', () => {
    const look = computePose(DEFAULT_PET_SPEC, { ...base(), gazeX: 0.5, gazeY: 0.3 })
    expect(look.pupilOffsetX).toBeGreaterThan(0)
    expect(look.headRotY).toBeGreaterThan(0)
  })

  it('happy 蹦跳：isBouncing 时输出周期小跳，关闭时为零', () => {
    const t = Math.PI / (2 * DEFAULT_PET_SPEC.motion.bounce.freq) // sin=1 峰值
    const bounce = computePose(DEFAULT_PET_SPEC, { ...base(), mood: 'happy', isBouncing: true, t })
    expect(bounce.bounceY).toBeCloseTo(DEFAULT_PET_SPEC.motion.bounce.amp)
    expect(bounce.rootPosY).toBeCloseTo(DEFAULT_PET_SPEC.motion.bounce.amp)
    const idle = computePose(DEFAULT_PET_SPEC, { ...base(), mood: 'happy', isBouncing: false, t })
    expect(idle.bounceY).toBeCloseTo(0)
    expect(idle.rootPosY).toBeCloseTo(0)
  })

  it('sleepy 揉眼：isRubbingEyes 时手臂抬起 + 眯眼加重；关闭时归零', () => {
    const rub = computePose(DEFAULT_PET_SPEC, {
      ...base(), mood: 'sleepy', arousal: 0.12, isRubbingEyes: true, t: Math.PI / 14,
    })
    const { rub: rubCfg } = DEFAULT_PET_SPEC.motion
    expect(rub.armLift).toBeGreaterThan(rubCfg.armLiftBase * 0.5)
    expect(rub.armLift).toBeLessThanOrEqual(rubCfg.armLiftBase + rubCfg.armLiftAmp + 1e-9)
    expect(rub.eyeScaleYL).toBeLessThan(rubCfg.eyeSquint + 1e-9)
    const noRub = computePose(DEFAULT_PET_SPEC, { ...base(), mood: 'sleepy', arousal: 0.12, isRubbingEyes: false })
    expect(noRub.armLift).toBeCloseTo(0)
    // 不揉眼时 armLift 不应为负数（数值卫生）
    expect(rub.armLift).toBeGreaterThanOrEqual(0)
  })
})

describe('petPose —— 交互纯函数（部位热区 / 手势 / 惯性 / 伪随机）', () => {
  it('部位热区：顶部=头 / 中段=背 / 底部=尾（含越界钳制）', () => {
    const { headBelow, tailAbove } = PET_INTERACTION.region
    expect(regionFromPoint(0)).toBe('head')
    expect(regionFromPoint(headBelow - 0.01)).toBe('head')
    expect(regionFromPoint(headBelow + 0.01)).toBe('back')
    expect(regionFromPoint(tailAbove - 0.01)).toBe('back')
    expect(regionFromPoint(tailAbove + 0.01)).toBe('tail')
    expect(regionFromPoint(1)).toBe('tail')
    // 越界钳制（不抛错，落在首尾段）
    expect(regionFromPoint(-5)).toBe('head')
    expect(regionFromPoint(5)).toBe('tail')
  })

  it('手势判别：小位移+短时长=轻点；位移或时长超阈值=拖动', () => {
    expect(classifyGesture(0, 0, 50)).toBe('tap')
    expect(classifyGesture(PET_INTERACTION.tapMaxMove, 0, PET_INTERACTION.tapMaxMs)).toBe('tap')
    expect(classifyGesture(PET_INTERACTION.tapMaxMove + 1, 0, 100)).toBe('drag')
    expect(classifyGesture(0, 0, PET_INTERACTION.tapMaxMs + 1)).toBe('drag')
  })

  it('惯性：确定性 + 摩擦减速 + 静置判定 + 触边贴边回弹 + dt 封顶', () => {
    const cfg = PET_INTERACTION.inertia
    const bounds = { minX: 0, minY: 0, maxX: 1000, maxY: 1000 }
    const body = { x: 100, y: 100, vx: 600, vy: 0 }
    const a = stepInertia(body, 1 / 60, bounds, cfg)
    const b = stepInertia(body, 1 / 60, bounds, cfg)
    expect(a).toEqual(b) // 确定性：同输入同输出
    expect(a.body.x).toBeGreaterThan(body.x) // 沿速度滑行
    expect(Math.abs(a.body.vx)).toBeLessThan(Math.abs(body.vx)) // 摩擦减速
    expect(a.resting).toBe(false) // 高速不停

    // 极低速 → 静置且速度归零
    const slow = stepInertia({ x: 10, y: 10, vx: cfg.minSpeed - 1, vy: 0 }, 1 / 60, bounds, cfg)
    expect(slow.resting).toBe(true)
    expect(slow.body.vx).toBe(0)

    // 触边：右越界 → 贴边到 maxX，速度反向（回弹）
    const hit = stepInertia({ x: 995, y: 500, vx: 600, vy: 0 }, 1 / 60, bounds, cfg)
    expect(hit.body.x).toBe(bounds.maxX)
    expect(hit.body.vx).toBeLessThanOrEqual(0)

    // dt 封顶：后台巨步不会瞬移出界
    const huge = stepInertia({ x: 0, y: 0, vx: 100000, vy: 0 }, 5, bounds, cfg)
    expect(huge.body.x).toBeLessThanOrEqual(bounds.maxX)
  })

  it('确定性伪随机：同 seed 同序列，值域 [0,1)，异 seed 不同序列', () => {
    const seq = (seed: number) => {
      const r = createSeededRandom(seed)
      return [r(), r(), r(), r(), r()]
    }
    expect(seq(12345)).toEqual(seq(12345))
    for (const v of seq(12345)) {
      expect(v).toBeGreaterThanOrEqual(0)
      expect(v).toBeLessThan(1)
    }
    expect(seq(999)).not.toEqual(seq(12345))
  })

  it('部位点击冲量：头=点头 / 尾=甩尾 / 背=侧倾；无点击为零；确定性且随时间衰减', () => {
    const noPoke = computePose(DEFAULT_PET_SPEC, base())
    expect(noPoke.pokeHeadRotX).toBe(0)
    expect(noPoke.pokeLeanRotZ).toBe(0)
    expect(noPoke.pokeTailBoost).toBe(0)

    const head = computePose(DEFAULT_PET_SPEC, { ...base(), poke: { region: 'head', elapsed: 0.02 } })
    expect(head.pokeHeadRotX).toBeGreaterThan(0)
    expect(head.headRotX).toBeGreaterThan(noPoke.headRotX)

    // 尾：t=0 时基线摆动为 0 → tailWag 即冲量本身
    const tail = computePose(DEFAULT_PET_SPEC, {
      ...base(),
      poke: { region: 'tail', elapsed: Math.PI / 2 / DEFAULT_PET_SPEC.motion.poke.freq },
    })
    expect(tail.pokeTailBoost).toBeGreaterThan(0)
    expect(tail.tailWag).toBeCloseTo(tail.pokeTailBoost)

    // 背：侧倾分量 > 0（已折叠进 rootRotZ）
    const back = computePose(DEFAULT_PET_SPEC, {
      ...base(),
      poke: { region: 'back', elapsed: Math.PI / 2 / DEFAULT_PET_SPEC.motion.poke.freq },
    })
    expect(back.pokeLeanRotZ).toBeGreaterThan(0)
    expect(back.rootRotZ).toBeGreaterThan(0)

    // 确定性 + 衰减：2 秒后冲量幅值明显小于起始
    const again = computePose(DEFAULT_PET_SPEC, { ...base(), poke: { region: 'head', elapsed: 0.02 } })
    expect(again.pokeHeadRotX).toBe(head.pokeHeadRotX)
    const decayed = computePose(DEFAULT_PET_SPEC, { ...base(), poke: { region: 'head', elapsed: 2 } })
    expect(Math.abs(decayed.pokeHeadRotX)).toBeLessThan(Math.abs(head.pokeHeadRotX))
  })
})