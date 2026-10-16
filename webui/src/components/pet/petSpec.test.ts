import { beforeAll, describe, expect, it } from 'vitest'
import {
  BEAST_EYE_ANCHORS,
  DEFAULT_PET_SPEC,
  ELF_SPECS,
  HUMANOID_EYE_ANCHORS,
  HUMANOID_FRINGE,
  PET_CAMERA,
  PET_EMOTION_ZOOM,
  PET_HOVER_FOCUS,
  PET_HOVER_ZOOM,
  PET_INTERACTION,
  PET_GLOW,
  PET_LIGHTING,
  PET_PERF_BUDGET,
  PET_QUALITY,
  PET_QUALITY_ORDER,
  PET_TOON_LOW_FLOOR,
  PET_TOON_RAMP,
  PET_TOON_RAMP_WIDTH,
  PET_EYE,
  cameraZoomTarget,
  createPetTierState,
  estimateExposure,
  visibleExposureProfile,
  eyeAnchorsFor,
  eyeLayout,
  fringeBottomY,
  getPetModelSpec,
  haloGlowOpacity,
  humanoidEyeTopY,
  humanoidSideLocks,
  humanoidWisps,
  isPetSpecValid,
  mergeAppearance,
  stepCameraZoom,
  stepDownQuality,
  stepPetTier,
  stepUpQuality,
  toonBandFor,
  toonRampBytes,
  validatePetSpec,
  zoomedCameraPosition,
} from './petSpec'
import type { PetMood3D } from './petContract'
import petSvgSource from './petSvg.ts?raw'

const ALL_MOOD3D: PetMood3D[] = ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad']

/** PetSvg 源码（剥离注释）：源码扫描类守卫统一走它，避免注释里的示例代码被当成真实代码 */
const petSvgCode = petSvgSource
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .split('\n')
  .filter(line => !line.trimStart().startsWith('//'))
  .join('\n')
/** V2.1 基线皮肤：猫 + 4 精灵，共 5 款，全部走数据驱动 3D 造型 */
const ALL_SKINS: ReadonlyArray<[string, 'beast' | 'humanoid', string]> = [
  ['cat', 'beast', '猫'],
  ['blue', 'humanoid', '雏菊精灵'],
  ['pink', 'humanoid', '樱花精灵'],
  ['purple', 'humanoid', '薰衣草精灵'],
  ['green', 'humanoid', '四叶草精灵'],
]

describe('petSpec —— 规格数据层（数据驱动 + 诚实渲染）', () => {
  it('默认规格合法（零错误）', () => {
    expect(isPetSpecValid(DEFAULT_PET_SPEC)).toBe(true)
  })

  it('emotionPoses 覆盖全部 3D 心情（新增心情漏配 → 校验失败）', () => {
    const errs = validatePetSpec(DEFAULT_PET_SPEC)
    expect(errs.filter((e) => e.includes('缺少心情'))).toEqual([])
    for (const m of ALL_MOOD3D) {
      expect(DEFAULT_PET_SPEC.emotionPoses[m]).toBeDefined()
    }
  })

  it('坏 Spec 被校验拒绝（尺寸为 0 / 嘴型非法 / 腮红越界）', () => {
    const bad = { ...DEFAULT_PET_SPEC }
    // 1) 尺寸为 0
    const badProps = { ...bad.proportions, bodyRadius: 0 }
    expect(validatePetSpec({ ...bad, proportions: badProps })).toContain(
      'proportions.bodyRadius 必须为正数（当前 0）',
    )
    // 2) 缺失心情
    const noSad = { ...bad.emotionPoses }
    delete (noSad as Partial<Record<PetMood3D, unknown>>).sad
    expect(validatePetSpec({ ...bad, emotionPoses: noSad as never })).toContain(
      "emotionPoses 缺少心情 'sad'",
    )
    // 3) 腮红越界
    const badBlush = structuredClone(DEFAULT_PET_SPEC)
    badBlush.emotionPoses.happy.blush.opacity = 1.5
    expect(validatePetSpec(badBlush)).toContain("emotionPoses['happy'].blush.opacity 须在 0..1")
  })

  it('Data 驱动：修改运动参数可影响规格（皮肤 override 语义存在）', () => {
    const faster = { ...DEFAULT_PET_SPEC }
    faster.motion = { ...faster.motion, breath: { freq: 3.5, amp: 0.05 } }
    expect(isPetSpecValid(faster)).toBe(true)
    expect(faster.motion.breath.freq).toBeGreaterThan(DEFAULT_PET_SPEC.motion.breath.freq)
  })

  // —— V2.1-P3 扩充：4 精灵皮肤 + cat 基线全量校验 ——

  it.each(ALL_SKINS)('皮肤 %s (%s) · getPetModelSpec 解析无兜底（非未知 id 不回退 DEFAULT）', (id, kind, label) => {
    const spec = getPetModelSpec(id)
    expect(spec.id, `${label} 规格 id 应与查询 key 一致`).toBe(id)
    expect(spec.kind, `${label} 期望 kind=${kind}`).toBe(kind)
    if (id !== 'cat') {
      // 精灵皮肤：必须是 ELF_SPECS 内的引用（data-driven 热插拔单一来源）
      expect(ELF_SPECS[id]).toBe(spec)
    }
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · validatePetSpec 全通过（0 错误）', (id, _kind, label) => {
    const spec = getPetModelSpec(id)
    const errs = validatePetSpec(spec)
    expect(errs, `${label} 校验错误列表应为空`).toEqual([])
    expect(isPetSpecValid(spec)).toBe(true)
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · emotionPoses 覆盖全部 6 种心情', (id, _kind, label) => {
    const spec = getPetModelSpec(id)
    for (const m of ALL_MOOD3D) {
      expect(spec.emotionPoses[m], `${label} 缺少心情 pose: ${m}`).toBeDefined()
    }
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · kind 与 appearance 字段组协调（kind-aware 校验不触发假阳性）', (id, kind, label) => {
    const spec = getPetModelSpec(id)
    // 兽型 appearance 可能缺省，按渲染路径用 mergeAppearance 合并默认值
    const app = mergeAppearance(spec)
    if (kind === 'humanoid') {
      expect(app.hairStyle, `${label} 人型应声明 hairstyle`).toBeDefined()
      expect(app.dressStyle, `${label} 人型应声明 dressStyle`).toBeDefined()
      expect(app.earStyle, `${label} 人型应声明 earStyle`).toBe('elf')
    }
    // appearance 颜色通道为 16 进制正整数（0x000000 ~ 0xFFFFFF）
    const colorKeys = ['furColor', 'furAccent', 'skin', 'dressColor', 'dressAccent', 'shoeColor'] as const
    for (const k of colorKeys) {
      const v = app[k] as number
      expect(Number.isInteger(v), `${label} appearance.${k}=${v} 必须是整数`).toBe(true)
      expect(v, `${label} appearance.${k} 必须在 [0, 0xFFFFFF]`).toBeGreaterThanOrEqual(0)
      expect(v, `${label} appearance.${k} 必须在 [0, 0xFFFFFF]`).toBeLessThanOrEqual(0xff_ff_ff)
    }
  })
})

describe('petSpec —— 交互规格 + 帧率感知降级判定（纯函数）', () => {
  it('部位点击规格存在且交互常量自洽（头/背/尾分界有序、阈值正数）', () => {
    expect(DEFAULT_PET_SPEC.motion.poke.headAmp).toBeGreaterThan(0)
    expect(DEFAULT_PET_SPEC.motion.poke.backAmp).toBeGreaterThan(0)
    expect(DEFAULT_PET_SPEC.motion.poke.tailAmp).toBeGreaterThan(0)
    expect(DEFAULT_PET_SPEC.motion.poke.decay).toBeGreaterThan(0)
    expect(PET_INTERACTION.region.headBelow).toBeLessThan(PET_INTERACTION.region.tailAbove)
    expect(PET_INTERACTION.tapMaxMove).toBeGreaterThan(0)
    expect(PET_INTERACTION.inertia.minSpeed).toBeGreaterThan(0)
    expect(PET_INTERACTION.inertia.restitution).toBeGreaterThanOrEqual(0)
  })

  it('精灵皮肤继承 poke 规格（数据驱动，未漏配）', () => {
    for (const id of ['blue', 'pink', 'purple', 'green']) {
      expect(getPetModelSpec(id).motion.poke.tailAmp).toBeGreaterThan(0)
    }
  })

  it('档位→配置映射表：三档单调、medium 保住阴影、low 关阴影且不再自适应', () => {
    // 单调性：越高档越不省（像素比与粒子产出随档位递减）
    const tiers = PET_QUALITY_ORDER
    expect(tiers).toEqual(['high', 'medium', 'low'])
    for (let i = 1; i < tiers.length; i += 1) {
      const hi = PET_QUALITY[tiers[i - 1]]
      const lo = PET_QUALITY[tiers[i]]
      expect(hi.maxPixelRatio).toBeGreaterThan(lo.maxPixelRatio)
      expect(hi.particleBudget).toBeGreaterThan(lo.particleBudget)
    }
    // 中间档的**存在理由**：保住阴影（最大视觉收益），只降抗锯齿/采样率/阴影分辨率
    expect(PET_QUALITY.medium.shadows).toBe(true)
    expect(PET_QUALITY.medium.antialias).toBe(false)
    expect(PET_QUALITY.medium.maxPixelRatio).toBeLessThan(PET_QUALITY.high.maxPixelRatio)
    expect(PET_QUALITY.medium.shadowMapSize).toBeLessThan(PET_QUALITY.high.shadowMapSize)
    // 省电档：无阴影、无抗锯齿、采样率 1、不再自适应
    expect(PET_QUALITY.low.shadows).toBe(false)
    expect(PET_QUALITY.low.antialias).toBe(false)
    expect(PET_QUALITY.low.maxPixelRatio).toBe(1)
    expect(PET_QUALITY.low.adaptiveDegrade).toBe(false)
    expect(PET_QUALITY.high.adaptiveDegrade).toBe(true)
    // 阴影贴图边长只对有阴影的档位有意义
    for (const tier of tiers) {
      if (PET_QUALITY[tier].shadows) expect(PET_QUALITY[tier].shadowMapSize).toBeGreaterThan(0)
    }
  })

  it('档位升降：逐级且不越界（high 升不上去 / low 降不下来）', () => {
    expect(stepDownQuality('high')).toBe('medium')
    expect(stepDownQuality('medium')).toBe('low')
    expect(stepDownQuality('low')).toBe('low')
    expect(stepUpQuality('low')).toBe('medium')
    expect(stepUpQuality('medium')).toBe('high')
    expect(stepUpQuality('high')).toBe('high')
  })

  it('帧率自适应：连续低帧达 streak 逐级降档；中间带保持；连续高帧达 streak 逐级升档', () => {
    const cfg = PET_PERF_BUDGET
    let s = createPetTierState('high')
    s = stepPetTier(s, cfg.lowFps - 5)
    expect(s.tier).toBe('high') // 第 1 个低帧窗口未达 streak
    expect(s.lowStreak).toBe(1)
    s = stepPetTier(s, cfg.lowFps - 5)
    expect(s.tier).toBe('medium') // 达 streak → 降**一档**（不是直接掉到 low）
    // 中间带：保持当前档位并清零计数（避免抖动切换）
    const mid = stepPetTier(s, (cfg.lowFps + cfg.recoverFps) / 2)
    expect(mid.tier).toBe('medium')
    expect(mid.lowStreak).toBe(0)
    // 继续掉帧 → 再降一档
    let low = stepPetTier(mid, cfg.lowFps - 5)
    low = stepPetTier(low, cfg.lowFps - 5)
    expect(low.tier).toBe('low')
    // 连续高帧达 streak → 逐级升回
    let r = stepPetTier(low, cfg.recoverFps + 5)
    expect(r.tier).toBe('low') // 尚未达 streak
    r = stepPetTier(r, cfg.recoverFps + 5)
    expect(r.tier).toBe('medium') // 达 streak → 升一档
    r = stepPetTier(stepPetTier(r, cfg.recoverFps + 5), cfg.recoverFps + 5)
    expect(r.tier).toBe('high')
  })

  it('未降档时中间/高帧均保持档位；low 起点的升档也受 streak 约束', () => {
    let s = createPetTierState('high')
    s = stepPetTier(s, PET_PERF_BUDGET.recoverFps + 5)
    expect(s.tier).toBe('high')
    s = stepPetTier(s, (PET_PERF_BUDGET.lowFps + PET_PERF_BUDGET.recoverFps) / 2)
    expect(s.tier).toBe('high')
    // low 起点：高帧同样要连续达 streak 才升档（升档与降档同一套去抖）
    let l = createPetTierState('low')
    l = stepPetTier(l, PET_PERF_BUDGET.recoverFps + 5)
    expect(l.tier).toBe('low')
    l = stepPetTier(l, PET_PERF_BUDGET.recoverFps + 5)
    expect(l.tier).toBe('medium')
  })

  it('渲染器不持有档位字面量判断（档位开关的单一事实源 = PET_QUALITY 表）', () => {
    // 为什么需要它：档位开关一旦被写成 `quality === 'high'`，新增档位就会被**静默忽略**
    // （正是「无中间档」那类缺陷的成因）。这里钉住「渲染器只查表」这条不变量。
    const code = petSvgCode
    const offenders = code
      .split('\n')
      .map((line, i) => [i + 1, line] as const)
      .filter(([, line]) => /quality\s*[!=]==?\s*['"]/.test(line))
    expect(offenders, `PetSvg 出现档位字面量比较（应改为查 PET_QUALITY 表）：${JSON.stringify(offenders)}`).toEqual([])
    // 反向自检：扫描规则必须真的能命中（否则本守卫是空转的）
    expect(/quality\s*[!=]==?\s*['"]/.test("if (this.quality === 'high') {}")).toBe(true)
    expect(code).toContain('PET_QUALITY[')
  })

  it('柔光层：光晕必须溢出环外、比硬边环淡，且与光环同步亮灭', () => {
    // 半径倍数 > 1 才叫「溢出环外」的柔光（=1 与环同大看不出光晕，<1 会缩进环内）
    expect(PET_GLOW.haloRadiusRatio).toBeGreaterThan(1)
    // 光晕比硬边环淡：否则整圈过曝成白团
    expect(PET_GLOW.haloOpacityRatio).toBeGreaterThan(0)
    expect(PET_GLOW.haloOpacityRatio).toBeLessThan(1)
    // 映射：环不亮则无光晕（杜绝「无环有光」的鬼影）
    expect(haloGlowOpacity(0)).toBe(0)
    expect(haloGlowOpacity(0.75)).toBeCloseTo(0.75 * PET_GLOW.haloOpacityRatio, 10)
    expect(haloGlowOpacity(1)).toBeCloseTo(PET_GLOW.haloOpacityRatio, 10)
    // 异常输入夹到 [0,1]，不放大成过曝白团
    expect(haloGlowOpacity(3)).toBeCloseTo(PET_GLOW.haloOpacityRatio, 10)
    expect(haloGlowOpacity(-1)).toBe(0)
    expect(haloGlowOpacity(Number.NaN)).toBe(0)
    // 与 petPose 的真实取值（working 0.55 / excited 0.75 / dancing 0.85）对齐：可见但不刺眼
    expect(haloGlowOpacity(0.55)).toBeGreaterThan(0.2)
    expect(haloGlowOpacity(0.85)).toBeLessThan(0.5)
  })

  it('光环柔光的不透明度必须经 haloGlowOpacity 映射（不得直接抄光环值）', () => {
    // 回归背景：直接 `opacity = p.haloOpacity` 会让光晕与硬边环同样亮 → 过曝白团，
    // 且柔光层失去「比环淡」的语义。这里钉住「驱动值来自纯函数」。
    const driver = petSvgCode.split('\n').find(line => line.includes('haloGlow.setAttribute'))
    expect(driver, 'PetSvg 未驱动光环柔光不透明度（柔光层形同虚设）').toBeTruthy()
    expect(driver).toContain('haloGlowOpacity(')
  })

  it('头饰高度 = 版式基准 + 浮动偏移（回归：直接赋值把头饰拽进头里，整件不可见）', () => {
    // 真实缺陷：`computePose.flowerPosY` 只是 ±0.012 的呼吸偏移，而 applyPose 曾
    // 把它当绝对 y 赋值 → 头饰每帧被拽到头部球心，5 款皮肤的头饰/花环**全部不可见**。
    const driver = petSvgCode.split('\n').find(line => line.includes('ornament.setAttribute'))
    expect(driver, 'PetSvg 未驱动头饰高度').toBeTruthy()
    // 高度必须是「构建期定死的基准 + 逐帧偏移」，而非把偏移直接当坐标
    expect(driver, '头饰高度必须是「基准 + 偏移」').toContain('ornamentBaseTransform()')
    expect(driver, '头饰偏移必须来自 computePose.flowerPosY 的 translate(0 …)').toContain('translate(0 ')
    expect(driver).toContain('flowerPosY')
  })
})

describe('petSpec —— 相机框景（取景即数据：FOV/机位/视点全部入表）', () => {
  it('取景表覆盖两个建模分支，且取值有限、注视点在机位前方', () => {
    for (const kind of ['beast', 'humanoid'] as const) {
      const cam = PET_CAMERA[kind]
      expect(cam, `缺 ${kind} 的取景`).toBeTruthy()
      expect(cam.fov).toBeGreaterThan(10)
      expect(cam.fov).toBeLessThan(90)
      for (const v of [...cam.position, ...cam.lookAt]) expect(Number.isFinite(v)).toBe(true)
      // 相机必须在场景正前方（+Z）看向原点附近，否则模型会背对镜头
      expect(cam.position[2]).toBeGreaterThan(0)
      expect(cam.lookAt[2]).toBe(0)
    }
  })

  it('渲染器不持有画布尺寸字面量（取景的单一事实源 = PET_SVG_SIZE）', () => {
    // 旧守卫钉的是 Three.js 相机：构造器曾写死 `new PerspectiveCamera(40,…)` 与
    // `position.set(0, 1.18, 5.2)` —— 换皮肤改不了取景。SVG 渲染器没有相机，
    // 「取景」= viewBox：画布边长必须来自 PET_SVG_SIZE，不得写成裸数字；
    // 且不得把 WebGL / Three.js 依赖带回来（这正是本次迁移要终结的路径）。
    const viewBox = petSvgCode.split('\n').filter(line => line.includes('viewBox'))
    expect(viewBox).toHaveLength(1)
    expect(viewBox[0], 'viewBox 必须读取 PET_SVG_SIZE').toContain('PET_SVG_SIZE')
    const hardcoded = petSvgCode
      .split('\n')
      .map((line, i) => [i + 1, line.trim()] as const)
      .filter(([, line]) => /viewBox\s*:\s*['"]0 0 \d/.test(line))
    expect(hardcoded, `画布尺寸被写死：${JSON.stringify(hardcoded)}`).toEqual([])
    // 反向自检：扫描规则必须真能命中（否则守卫空转）
    expect(/viewBox\s*:\s*['"]0 0 \d/.test("viewBox: '0 0 200 200'")).toBe(true)
    // 渲染器不得依赖 WebGL / Three.js（迁移的硬边界）
    for (const banned of ['THREE', 'WebGL', 'webgl']) {
      expect(petSvgCode.includes(banned), `渲染器不得出现 ${banned}`).toBe(false)
    }
  })

  it('情绪推近/拉远：六种心情齐备，兴奋拉近、困倦退开，专注/常态保持基准', () => {
    for (const m of ALL_MOOD3D) {
      expect(PET_EMOTION_ZOOM[m], `缺 ${m} 的取景`).toBeTruthy()
      expect(PET_EMOTION_ZOOM[m]).toBeGreaterThan(0.8)
      expect(PET_EMOTION_ZOOM[m]).toBeLessThan(1.3)
    }
    expect(cameraZoomTarget('excited')).toBeGreaterThan(1)
    expect(cameraZoomTarget('happy')).toBeGreaterThan(1)
    expect(cameraZoomTarget('sleepy')).toBeLessThan(1)
    expect(cameraZoomTarget('sad')).toBeLessThan(1)
    expect(cameraZoomTarget('working')).toBe(1)
    expect(cameraZoomTarget('normal')).toBe(1)
    // 悬停 = 在情绪基线上线性叠加（不是相乘：边界可控，永不怼脸）
    expect(cameraZoomTarget('normal', 1)).toBeCloseTo(PET_HOVER_ZOOM, 10)
    expect(cameraZoomTarget('normal', 0.5)).toBeCloseTo(1 + (PET_HOVER_ZOOM - 1) * 0.5, 10)
    expect(cameraZoomTarget('excited', 1)).toBeCloseTo(PET_EMOTION_ZOOM.excited * PET_HOVER_ZOOM, 10)
    // 越界/非法输入不放大成过近
    expect(cameraZoomTarget('normal', 5)).toBe(cameraZoomTarget('normal', 1))
    expect(cameraZoomTarget('normal', -5)).toBe(cameraZoomTarget('normal', 0))
    expect(cameraZoomTarget('normal', Number.NaN)).toBe(1)
  })

  it('推近逼近：帧率无关、永不越过、坏输入不炸', () => {
    // 两个 0.05s 步长 ≈ 一个 0.1s 步长（指数逼近的帧率无关性）
    const twoSteps = stepCameraZoom(stepCameraZoom(1, 1.1, 0.05), 1.1, 0.05)
    const oneStep = stepCameraZoom(1, 1.1, 0.1)
    expect(Math.abs(twoSteps - oneStep)).toBeLessThan(0.002)
    // 向目标逼近且不越过
    let z = 1
    for (let i = 0; i < 60; i++) z = stepCameraZoom(z, 0.93, 1 / 60)
    expect(z).toBeGreaterThanOrEqual(0.93)
    expect(z).toBeLessThan(1)
    // 足够时间后收敛到目标（浮点收尾：不再「永远差一点点」）
    expect(stepCameraZoom(1, 1.1, 5)).toBe(1.1)
    // 坏输入：NaN 直接落到目标（不把相机丢进 NaN 空间）
    expect(stepCameraZoom(Number.NaN, 1.05, 0.016)).toBe(1.05)
    expect(stepCameraZoom(1, Number.NaN, 0.016)).toBe(Number.NaN)
    // 负 dt/rate 不倒退
    expect(stepCameraZoom(1, 1.1, -1)).toBe(1)
  })

  it('绕注视点推近：注视点不动、方向共线、距离按 1/zoom 缩短', () => {
    const cam = PET_CAMERA.humanoid
    expect(zoomedCameraPosition(cam, 1)).toEqual([...cam.position])
    const near = zoomedCameraPosition(cam, 2)
    const d = (p: readonly number[]) => Math.hypot(p[0] - cam.lookAt[0], p[1] - cam.lookAt[1], p[2] - cam.lookAt[2])
    expect(d(near)).toBeCloseTo(d(cam.position) / 2, 10)
    // 共线：推近只是「走近两步」，不是转头（叉积为 0）
    const a = [cam.position[0] - cam.lookAt[0], cam.position[1] - cam.lookAt[1], cam.position[2] - cam.lookAt[2]]
    const b = [near[0] - cam.lookAt[0], near[1] - cam.lookAt[1], near[2] - cam.lookAt[2]]
    const cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    expect(Math.max(...cross.map(Math.abs))).toBeLessThan(1e-9)
    // 非法 zoom 退化为基准机位（不产生 NaN 坐标）
    expect(zoomedCameraPosition(cam, Number.NaN)).toEqual([...cam.position])
    expect(zoomedCameraPosition(cam, 0)).toEqual([...cam.position])
  })

  it('悬停聚焦增益有界（不会把瞳孔放大到穿出眼白）', () => {
    expect(PET_HOVER_FOCUS.gazeGain).toBeGreaterThan(0)
    expect(PET_HOVER_FOCUS.gazeGain).toBeLessThan(1)
    expect(PET_HOVER_FOCUS.pupilGain).toBeGreaterThan(0)
    expect(PET_HOVER_FOCUS.pupilScaleGain).toBeGreaterThan(0)
    expect(PET_HOVER_FOCUS.pupilScaleGain).toBeLessThan(0.3)
    expect(PET_HOVER_ZOOM).toBeGreaterThan(1)
    expect(PET_HOVER_ZOOM).toBeLessThan(1.15)
  })
})

/**
 * @vitest-environment happy-dom
 */
describe('petSpec · PetSvg 构造不抛错（诚实渲染门禁）', () => {
  // SVG 渲染器是纯 DOM：不依赖 WebGL，故不再 skip，对全部皮肤真跑断言。
  let PetSvgCtor: typeof import('./petSvg').PetSvg | null = null
  beforeAll(async () => {
    PetSvgCtor = (await import('./petSvg')).PetSvg
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · PetSvg 构造不抛错', (id, _kind, label) => {
    const spec = getPetModelSpec(id)
    const container = document.createElement('div')
    document.body.appendChild(container)
    try {
      expect(() => {
        const inst = new PetSvgCtor!({ container, accent: spec.accent, spec, quality: 'low' })
        expect(container.querySelector('svg'), `${label} 必须真的插入 <svg>`).toBeTruthy()
        inst.dispose()
      }, `${label} PetSvg 构造不抛错`).not.toThrow()
    } finally {
      document.body.removeChild(container)
    }
  })
})

describe('眼部锚点：构建与动画的单一事实源', () => {
  it('两种形态各有独立锚点（回归：此前动画层用兽形坐标覆写人形，导致虹膜/高光错位）', () => {
    expect(eyeAnchorsFor('beast')).toBe(BEAST_EYE_ANCHORS)
    expect(eyeAnchorsFor('humanoid')).toBe(HUMANOID_EYE_ANCHORS)
    // 若两者相等，说明又退化成「一份坐标走天下」——那正是本次修复的缺陷
    expect(eyeAnchorsFor('humanoid')).not.toEqual(eyeAnchorsFor('beast'))
  })

  it('人形锚点与构建器的布局一致（眼距 ±0.16、高光 ±0.135/0.185）', () => {
    const h = eyeAnchorsFor('humanoid')
    expect(h.eyeL.x).toBeCloseTo(-0.16, 6)
    expect(h.eyeR.x).toBeCloseTo(0.16, 6)
    expect(h.pupilL.x).toBeCloseTo(-0.16, 6)
    expect(h.pupilR.x).toBeCloseTo(0.16, 6)
    expect(h.shineL.x).toBeCloseTo(-0.135, 6)
    expect(h.shineR.x).toBeCloseTo(0.185, 6)
    // 左右严格镜像对称（X 等值异号）
    expect(h.eyeL.x).toBeCloseTo(-h.eyeR.x, 6)
    expect(h.pupilL.x).toBeCloseTo(-h.pupilR.x, 6)
  })

  it('兽形锚点与旧版视觉等值（眼距 ±0.18、瞳孔 y=0.04、高光 y=0.095）', () => {
    const b = eyeAnchorsFor('beast')
    expect(b.eyeL.x).toBeCloseTo(-0.18, 6)
    expect(b.eyeR.x).toBeCloseTo(0.18, 6)
    expect(b.pupilL.y).toBeCloseTo(0.04, 6)
    expect(b.shineL.y).toBeCloseTo(0.095, 6)
    expect(b.shineR.x).toBeCloseTo(0.21, 6)
  })

  it('锚点全部为有限数（NaN 会让 applyPose 静默产出不可见网格）', () => {
    for (const kind of ['beast', 'humanoid'] as const) {
      const a = eyeAnchorsFor(kind)
      for (const key of ['eyeL', 'eyeR', 'pupilL', 'pupilR', 'shineL', 'shineR'] as const) {
        expect(Number.isFinite(a[key].x), `${kind}.${key}.x`).toBe(true)
        expect(Number.isFinite(a[key].y), `${kind}.${key}.y`).toBe(true)
      }
    }
  })
})

describe('人形发丝垂直布局：不得飘出头顶', () => {
  // 头部中心为本地 0，头顶 = +headRadius（由 spec 提供，不写死）
  const headRadius = getPetModelSpec('blue').proportions.headRadius

  it('鬓发整缕位于头部中心之下（回归：此前世界 y 2.15~2.41 飘在头顶之上）', () => {
    const locks = humanoidSideLocks()
    expect(locks).toHaveLength(6)
    for (const s of locks) {
      // 中心必须在头部中心之下
      expect(s.y, `鬓发中心 y=${s.y} 应在头部中心之下`).toBeLessThan(0)
      // 整缕的顶端也不得高过头顶
      const top = s.y + s.length / 2
      expect(top, `鬓发顶端 ${top.toFixed(3)} 不得高过头顶 ${headRadius}`).toBeLessThanOrEqual(headRadius)
    }
  })

  it('鬓发向下垂落（底端低于下巴区域），与注释「顺肩而下」一致', () => {
    for (const s of humanoidSideLocks()) {
      const bottom = s.y - s.length / 2
      expect(bottom, `鬓发底端 ${bottom.toFixed(3)} 应垂到下巴以下`).toBeLessThan(-0.5)
    }
  })

  it('飘丝环绕头部前方，且不飘出头顶', () => {
    const wisps = humanoidWisps()
    expect(wisps).toHaveLength(5)
    for (const s of wisps) {
      expect(s.y + s.length / 2, '飘丝顶端不得高过头顶').toBeLessThanOrEqual(headRadius)
      expect(s.y, '飘丝应在头部上半区环绕，而非坠到身体').toBeGreaterThan(-0.2)
    }
  })

  it('鬓发左右镜像对称（X 等值异号）', () => {
    const locks = humanoidSideLocks()
    // 按绝对值排序后逐项配对：左侧 -x 必须与右侧 +x 严格镜像
    const leftAbs = locks.filter((s) => s.x < 0).map((s) => Math.abs(s.x)).sort((a, b) => a - b)
    const rightAbs = locks.filter((s) => s.x > 0).map((s) => Math.abs(s.x)).sort((a, b) => a - b)
    expect(leftAbs).toHaveLength(3)
    expect(leftAbs).toEqual(rightAbs)
    for (const s of locks) expect(s.x).not.toBe(0)
  })
})

describe('卡通渐变档位：白色部件不得被压成中灰', () => {
  const lum = (hex: number) => ((hex >> 16) & 0xff) * 0.2126 + ((hex >> 8) & 0xff) * 0.7152 + (hex & 0xff) * 0.0722
  /** 灰度档位的 0..1 乘数 */
  const band = (hex: number) => lum(hex) / 255

  it('LOW 档不得低于下限（回归：此前 0x7a7a7a 仅 48% 亮度，把纯白内裙渲染成灰）', () => {
    expect(lum(PET_TOON_RAMP.low)).toBeGreaterThanOrEqual(PET_TOON_LOW_FLOOR)
  })

  it('三档单调递增（LOW < MID < HIGH），否则明暗分界会反转', () => {
    expect(lum(PET_TOON_RAMP.low)).toBeLessThan(lum(PET_TOON_RAMP.mid))
    expect(lum(PET_TOON_RAMP.mid)).toBeLessThanOrEqual(lum(PET_TOON_RAMP.high))
  })

  it('HIGH 为纯白（受光面必须能还原基色本身）', () => {
    expect(PET_TOON_RAMP.high).toBe(0xffffff)
  })

  it('纯白基色 × LOW 后仍属「亮部」而非中灰（≥ 0.7 相对亮度）', () => {
    // MeshToonMaterial 是基色 × 档位：白裙最暗处也不该掉到中灰
    const white = 0xffffff
    const shaded = Math.round(lum(white) * (lum(PET_TOON_RAMP.low) / 255))
    expect(shaded / 255).toBeGreaterThanOrEqual(0.7)
  })

  it('三档均为合法 24 位颜色', () => {
    for (const v of [PET_TOON_RAMP.low, PET_TOON_RAMP.mid, PET_TOON_RAMP.high]) {
      expect(Number.isInteger(v)).toBe(true)
      expect(v).toBeGreaterThanOrEqual(0)
      expect(v).toBeLessThanOrEqual(0xffffff)
    }
  })

  it('贴图字节布局必须是 24 字节 RGBA，且受光面可达的三档落在后半段', () => {
    const bytes = toonRampBytes()
    // WIDTH(6) texel × 4 字节 = 24；退回 RGB 的 18 字节会让贴图不完整 → 取样恒 0 → 平行光全灭
    expect(bytes).toHaveLength(PET_TOON_RAMP_WIDTH * 4)
    for (let i = 0; i < PET_TOON_RAMP_WIDTH; i++) {
      expect(bytes[i * 4 + 3], `texel ${i} 的 alpha 必须为 255`).toBe(255)
    }
    const red = (hex: number) => (hex >> 16) & 0xff
    // 死区（受光面采样不到）+ 第一个可达档都是 LOW，其后才是 MID / HIGH
    expect(bytes[0]).toBe(red(PET_TOON_RAMP.low))
    expect(bytes[12]).toBe(red(PET_TOON_RAMP.low))
    expect(bytes[16]).toBe(red(PET_TOON_RAMP.mid))
    expect(bytes[20]).toBe(red(PET_TOON_RAMP.high))
  })

  // ── R59 真根因守卫：受光面必须能取到整条渐变 ──
  // three 的取样坐标是 coord.x = dotNL*0.5 + 0.5（dotNL 已 saturate），受光面只能落在
  // [0.5, 1] 这半段。旧的 3 texel 布局下，受光面只能取到 0xee/0xff —— 可见明暗差仅 6.7%，
  // 白色部件因此平坦无立体感（也解释了历次「改 LOW 档 Δ=0」的费解结果）。
  it('受光面可达档位覆盖 LOW→MID→HIGH（回归：3 texel 时 LOW 对受光面是死代码）', () => {
    expect(toonBandFor(0)).toBeCloseTo(band(PET_TOON_RAMP.low), 6)
    expect(toonBandFor(0.2)).toBeCloseTo(band(PET_TOON_RAMP.low), 6)
    expect(toonBandFor(0.5)).toBeCloseTo(band(PET_TOON_RAMP.mid), 6)
    expect(toonBandFor(1)).toBeCloseTo(band(PET_TOON_RAMP.high), 6)
  })

  it('受光面可见的明暗差 ≥ 0.20（旧布局只有 0.067，等于没有立体感）', () => {
    // LOW 保持仓库既有的 0xc6（「不显脏」的既有结论），故可达区间是 0.776 → 1.0 = 0.224；
    // 旧 3 texel 布局下受光面只能取到 0.933/1.0 → 0.067。
    expect(band(PET_TOON_RAMP.high) - band(PET_TOON_RAMP.low)).toBeGreaterThanOrEqual(0.2)
    // 与「不显脏」的下限自洽：可达的最暗档不得低于 PET_TOON_LOW_FLOOR
    expect(band(PET_TOON_RAMP.low) * 255).toBeGreaterThanOrEqual(PET_TOON_LOW_FLOOR)
  })

  it('字节值在受光面可达段单调递增（暗→亮），否则卡通分界会反转', () => {
    const bytes = toonRampBytes()
    // 只检查**受光面可达**的三个 texel（#3/#4/#5）；前三个是背光死区，值相同
    const reds = [bytes[12], bytes[16], bytes[20]]
    expect(reds[0]).toBeLessThan(reds[1])
    expect(reds[1]).toBeLessThanOrEqual(reds[2])
  })
})

describe('灯光配平：三段渐变必须可见，不得被裁成死白', () => {
  const KINDS = ['humanoid', 'beast'] as const
  /** 主光正对面的纯白面（dotNL=1）的卡通口径总亮度上界 */
  const keyFaceTotal = (kind: (typeof KINDS)[number]) => {
    const k = PET_LIGHTING[kind].key.position
    const kl = Math.hypot(...k) || 1
    const e = estimateExposure(PET_LIGHTING[kind], [k[0] / kl, k[1] / kl, k[2] / kl])
    return e.ambient + e.direct
  }

  it.each(KINDS)('%s · 主光正对面不得明显过曝（上界 < 1.25）', (kind) => {
    // 卡通口径（R59 更正）：直接光 = 档位(dotNL) × 强度 / π，**没有**独立 N·L 因子。
    // 本式是**白反照率 + 白灯的上界**：人形（彩发/彩翼为主）实测「恰为 255」仅 3.7~15%，
    // 而上界 1.159 —— 偏松但仍有判别力（旧的兽形 1.371 会在这里被拦下）。
    expect(keyFaceTotal(kind)).toBeLessThan(1.25)
  })

  it('兽形 · 主光正对面不过曝（< 1.0；回归：旧 key=3.30 → 1.371，实测整只裁平）', () => {
    // 兽形通体白毛，上界与实测吻合（成立），故对它设紧界
    expect(keyFaceTotal('beast')).toBeLessThan(1.0)
  })

  it.each(KINDS)('%s · 单盏主光不得独自把纯白面推到裁切（≤ 0.95）', (kind) => {
    const key = PET_LIGHTING[kind].key
    // 回归背景：兽形 key=3.30 时单盏主光就是 1.05（dotNL=1 的正面），叠加任何补光必裁平。
    // 这条与其它灯无关，故不会被「补光调暗」这类改动掩盖。
    expect(key.intensity / Math.PI).toBeLessThanOrEqual(0.95)
  })

  it.each(KINDS)('%s · 背光面不压暗（渲染后不得比渐变 LOW 档下限更暗）', (kind) => {
    const L = PET_LIGHTING[kind]
    const k = L.key.position
    const kl = Math.hypot(...k) || 1
    // 背向主光的表面：dotNL 被 saturate 到 0，落到第一个可达档（LOW）
    const e = estimateExposure(L, [-k[0] / kl, -k[1] / kl, -k[2] / kl])
    const low = e.ambient + e.direct
    // R59 重新锚定：旧阈值「线性 ≥ 0.6」是在兽形 key=3.30 的过曝基线下定的，且与
    // 「不得裁平」互斥（见 PET_LIGHTING 注释里的扫描）。现锚定**已单独守卫**的
    // PET_TOON_LOW_FLOOR（0xb0 = 176，即 LOW 档允许的最暗值）：渲染字节不得低于它。
    const linToByte = (v: number) =>
      Math.round(255 * Math.min(1, Math.max(0, v)) ** (1 / 2.2))
    expect(linToByte(low)).toBeGreaterThanOrEqual(PET_TOON_LOW_FLOOR)
  })

  it.each(KINDS)('%s · 受光面能取到的明暗差可读（HIGH − LOW ≥ 0.20）', (kind) => {
    // 与渐变可达区间（0.776→1.0）同源：兽形 0.232 / 人形 0.30（旧兽形 0.16 且整只裁平）
    const L = PET_LIGHTING[kind]
    const k = L.key.position
    const kl = Math.hypot(...k) || 1
    const hi = estimateExposure(L, [k[0] / kl, k[1] / kl, k[2] / kl])
    const lo = estimateExposure(L, [-k[0] / kl, -k[1] / kl, -k[2] / kl])
    expect(hi.ambient + hi.direct - (lo.ambient + lo.direct)).toBeGreaterThanOrEqual(0.2)
  })

  it.each(KINDS)('%s · 灯光规格自洽（强度为正、位置为有限数、无 NaN 会静默灭灯）', (kind) => {
    const L = PET_LIGHTING[kind]
    const ints = [L.hemi.intensity, L.key.intensity, L.fill.intensity, L.rim.intensity, L.bottom.intensity]
    for (const v of ints) {
      expect(Number.isFinite(v)).toBe(true)
      expect(v).toBeGreaterThan(0)
    }
    expect(L.rim.distance).toBeGreaterThan(0)
    expect(L.bottom.distance).toBeGreaterThan(0)
    const positions = [L.key.position, L.fill.position, L.rim.position, L.bottom.position, L.front?.position]
    for (const p of positions) {
      if (!p) continue
      expect(p).toHaveLength(3)
      for (const c of p) expect(Number.isFinite(c)).toBe(true)
    }
    // 颜色通道为合法 24 位整数
    for (const c of [L.hemi.sky, L.hemi.ground, L.key.color, L.fill.color, L.bottom.color, L.front?.color]) {
      if (c === undefined) continue
      expect(Number.isInteger(c)).toBe(true)
      expect(c).toBeGreaterThanOrEqual(0)
      expect(c).toBeLessThanOrEqual(0xffffff)
    }
  })

  it('只有人形有正面补光（兽形镜头低、无需），且该差异是显式的', () => {
    expect(PET_LIGHTING.humanoid.front).toBeDefined()
    expect(PET_LIGHTING.beast.front).toBeUndefined()
  })

  it('estimateExposure 与法线方向单调一致（朝上更亮、朝下更暗）', () => {
    for (const kind of KINDS) {
      const L = PET_LIGHTING[kind]
      const up = estimateExposure(L, [0, 1, 0])
      const down = estimateExposure(L, [0, -1, 0])
      expect(up.ambient).toBeGreaterThan(down.ambient)
    }
  })

  // ── R59：可见半球（投影面积加权）的裁切/亮度上界 ──
  // 本模型是**白反照率 + 白灯的上界**：兽形（通体白毛）实测与上界吻合；
  // 人形 4 款皮肤以彩发/彩翼为主，实测「恰为 255」只占 3.7~15%，而模型上界 75% —— 偏松。
  // 故**紧界只设在实测缺陷所在的兽形**；人形只设硬天花板，其偏松部分登记为已知差距。
  it('兽形 · 可见半球被裁到纯白的面积 ≤ 15%（回归：旧 key=3.30 → 28.2%，实测整只裁平）', () => {
    const p = visibleExposureProfile(PET_LIGHTING.beast, PET_CAMERA.beast)
    expect(p.clippedShare).toBeLessThanOrEqual(0.15)
  })

  it.each(KINDS)('%s · 可见半球加权中位 ≤ 1.15（上界口径的硬天花板）', (kind) => {
    const p = visibleExposureProfile(PET_LIGHTING[kind], PET_CAMERA[kind])
    // 注：兽形实测线性中位由 1.000 降到复测值（见 PET_LIGHTING 注释）；人形 1.099
    expect(p.median).toBeLessThanOrEqual(1.15)
    // 且必须留出明暗余地：中位不得低到发暗
    expect(p.median).toBeGreaterThanOrEqual(0.35)
  })

  it('可见半球剖面是确定性纯函数（同输入同输出，无随机数）', () => {
    const a = visibleExposureProfile(PET_LIGHTING.beast, PET_CAMERA.beast)
    const b = visibleExposureProfile(PET_LIGHTING.beast, PET_CAMERA.beast)
    expect(a).toEqual(b)
  })
})

describe('眼部几何：按头围等比展开，且每层都必须穿出前一层', () => {
  const layoutFor = (id: string) => {
    const spec = getPetModelSpec(id)
    return {
      L: eyeLayout(spec.proportions.headRadius, spec.proportions.eyeScale),
      anchors: eyeAnchorsFor(spec.kind),
      headR: spec.proportions.headRadius,
      kind: spec.kind,
    }
  }

  it('半径严格由 headRadius × 比例 × eyeScale 展开（纯函数可复算）', () => {
    const L = eyeLayout(0.5, 2)
    expect(L.whiteR).toBeCloseTo(0.5 * PET_EYE.whiteOfHead * 2, 10)
    expect(L.irisR).toBeCloseTo(L.whiteR * PET_EYE.irisOfWhite, 10)
    expect(L.pupilR).toBeCloseTo(L.whiteR * PET_EYE.pupilOfWhite, 10)
    expect(L.eyeZ).toBeCloseTo(0.5 * PET_EYE.eyeZOfHead, 10)
  })

  it('非法 eyeScale 回退为 1（NaN/0/负数不得让眼型崩成不可见）', () => {
    for (const bad of [Number.NaN, 0, -3, Number.POSITIVE_INFINITY]) {
      const L = eyeLayout(0.5, bad)
      expect(Number.isFinite(L.whiteR)).toBe(true)
      expect(L.whiteR).toBeGreaterThan(0)
    }
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · 左右眼白不得重叠（回归：人形曾融成一坨白球盖住整张脸）', (id, _k, label) => {
    const { L, anchors } = layoutFor(id)
    const sep = Math.abs(anchors.eyeR.x - anchors.eyeL.x)
    expect(2 * L.whiteR, `${label} 两眼直径 ${(2 * L.whiteR).toFixed(3)} 应小于眼心距 ${sep}`).toBeLessThan(sep)
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · 眼白不得大到盖住脸（≤ 0.35 × headRadius）', (id, _k, label) => {
    const { L, headR } = layoutFor(id)
    expect(L.whiteR / headR, `${label} 眼白/头半径 = ${(L.whiteR / headR).toFixed(3)}`).toBeLessThanOrEqual(0.35)
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · 眼白既要凸出头球（可见）又不能整个浮在头前', (id, _k, label) => {
    const { L, headR } = layoutFor(id)
    expect(L.eyeZ, `${label} 眼白球心应在头球内`).toBeLessThan(headR)
    expect(L.eyeZ + L.whiteR, `${label} 眼白必须凸出头球表面，否则不可见`).toBeGreaterThan(headR)
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · 虹膜/瞳孔/高光必须穿出眼白表面（回归：人形虹膜曾被眼白吞没）', (id, _k, label) => {
    const { L } = layoutFor(id)
    const whiteFront = L.eyeZ + L.whiteR
    const layers: Array<[string, number, number]> = [
      ['虹膜', L.irisZ, L.irisR],
      ['瞳孔', L.pupilZ, L.pupilR],
      ['大高光', L.shineZ, L.shineR],
      ['小高光', L.shineSmallZ, L.shineSmallR],
    ]
    for (const [name, z, r] of layers) {
      expect(z + r, `${label} ${name} 前端 ${(z + r).toFixed(4)} 必须 > 眼白前端 ${whiteFront.toFixed(4)}`).toBeGreaterThan(whiteFront)
    }
  })

  it.each(ALL_SKINS)('皮肤 %s (%s) · 各层 Z 顺序正确（眼白 < 虹膜 < 瞳孔；小高光 < 大高光）', (id, _k, label) => {
    const { L } = layoutFor(id)
    expect(L.eyeZ, `${label}`).toBeLessThan(L.irisZ)
    expect(L.irisZ, `${label}`).toBeLessThan(L.pupilZ)
    expect(L.shineSmallZ, `${label}`).toBeLessThan(L.shineZ)
  })

  it('兽形视觉零回归：比例取自兽形现值（白 0.14/0.52、瞳 0.08/0.14、Z 0.42/0.52）', () => {
    const cat = getPetModelSpec('cat')
    const L = eyeLayout(cat.proportions.headRadius, cat.proportions.eyeScale)
    expect(L.whiteR).toBeCloseTo(0.14, 6)
    expect(L.irisR).toBeCloseTo(0.08, 6)
    expect(L.eyeZ).toBeCloseTo(0.42, 6)
    expect(L.irisZ).toBeCloseTo(0.52, 6)
    expect(L.shineZ).toBeCloseTo(0.57, 6)
  })
})

describe('前刘海：底缘必须高于眼白顶端（回归：刘海曾把眼睛切成半圆）', () => {
  it.each(ALL_SKINS.filter(([, kind]) => kind === 'humanoid'))(
    '皮肤 %s (%s) · 刘海底缘高于眼白顶端',
    (id, _k, label) => {
      const pr = getPetModelSpec(id).proportions
      const bottom = fringeBottomY(pr.headRadius)
      const eyeTop = humanoidEyeTopY(pr.headRadius, pr.eyeScale)
      expect(bottom, `${label} 刘海底缘 ${bottom.toFixed(4)} 必须 > 眼白顶端 ${eyeTop.toFixed(4)}`).toBeGreaterThan(eyeTop)
    },
  )

  it('切片数为正奇数（中央一片对齐眉心，左右对称）', () => {
    expect(HUMANOID_FRINGE.count).toBeGreaterThan(0)
    expect(HUMANOID_FRINGE.count % 2).toBe(1)
  })

  it('切片不得整体飘出头顶（顶缘 ≤ headRadius）', () => {
    const headR = 0.48
    const top = headR * HUMANOID_FRINGE.yOfHead + headR * HUMANOID_FRINGE.radiusOfHead * HUMANOID_FRINGE.stretchY
    expect(top).toBeLessThanOrEqual(headR)
  })

  it('横向覆盖足够宽（最外侧切片中心 |x| ≥ 0.5 × headRadius）', () => {
    const headR = 0.48
    const outerX = headR * HUMANOID_FRINGE.pitchOfHead * ((HUMANOID_FRINGE.count - 1) / 2)
    expect(outerX).toBeGreaterThanOrEqual(headR * 0.5)
  })

  it('规格数值自洽（半径/拉伸/压缩/间距均为正有限数）', () => {
    for (const v of [HUMANOID_FRINGE.radiusOfHead, HUMANOID_FRINGE.stretchY, HUMANOID_FRINGE.squashZ, HUMANOID_FRINGE.yOfHead, HUMANOID_FRINGE.pitchOfHead]) {
      expect(Number.isFinite(v)).toBe(true)
      expect(v).toBeGreaterThan(0)
    }
  })
})

describe('外观契约诚实性：声明了就必须生效', () => {
  const withAppearance = (patch: Record<string, unknown>) => ({
    ...DEFAULT_PET_SPEC,
    appearance: { ...mergeAppearance(DEFAULT_PET_SPEC), ...patch },
  })

  it('翼型三态全部被接受（butterfly/fairy/none 渲染器均有分支）', () => {
    for (const wingStyle of ['butterfly', 'fairy', 'none'] as const) {
      expect(validatePetSpec(withAppearance({ wingStyle })), `wingStyle=${wingStyle} 应被接受`).toEqual([])
    }
  })

  it('未实现的发型/裙型变体被明确拒绝（绝不静默降级为默认造型）', () => {
    // 此前校验放行 twin_tails/frill，但渲染器只读 long_wave/petal_dress → 静默忽略
    const hair = validatePetSpec(withAppearance({ hairStyle: 'twin_tails' }))
    expect(hair.join(';')).toMatch(/hairStyle.*尚未实现/)
    const dress = validatePetSpec(withAppearance({ dressStyle: 'frill' }))
    expect(dress.join(';')).toMatch(/dressStyle.*尚未实现/)
    // 已实现的默认变体仍然通过
    expect(validatePetSpec(withAppearance({ hairStyle: 'long_wave', dressStyle: 'petal_dress' }))).toEqual([])
  })

  it('hasBouquet 必须是布尔（此前完全未校验）', () => {
    expect(validatePetSpec(withAppearance({ hasBouquet: true }))).toEqual([])
    expect(validatePetSpec(withAppearance({ hasBouquet: false }))).toEqual([])
    const bad = validatePetSpec(withAppearance({ hasBouquet: 'yes' }))
    expect(bad.join(';')).toMatch(/hasBouquet/)
  })
})