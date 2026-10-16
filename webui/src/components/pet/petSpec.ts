/**
 * 桌宠模型规格（v3）— 「形状即数据」：全部 3D 建模与动画参数数据化。
 *
 * 设计动机（engineering-core 铁律：数据驱动热扩展）：
 * - 旧版 Pet3D 把身体比例、表情骨骼、动画频率全部硬编码在代码里，皮肤只能换 accent。
 * - 本模块把「一个模型长什么样、每种心情怎么动」抽成 PetModelSpec JSON：
 *   新增皮肤/表情 = 追加 Spec，**无需改渲染代码**（热插拔）。
 * - `validatePetSpec` 纯校验：坏 Spec 在测试期即失败，绝不带病渲染。
 */

import type { PetMood3D, PetRenderTier } from './petContract'

/** 建模几何比例（相对单位，渲染器缩放至实际尺寸） */
export interface ProportionsSpec {
  /** 身体半径（兽形用）/ 髋宽半径（人形用） */
  bodyRadius: number
  /** 身体 Y 缩放（扁/圆，兽形）；人形用 torsoLength */
  bodyScaleY: number
  /** 头部半径（兽形人形共用） */
  headRadius: number
  /** 耳高（锥体）—— 兽形必备；人形 = 0 不渲染 */
  earHeight: number
  /** 眼放大系数（>1 = 更大更萌，智慧感来源） */
  eyeScale: number
  /** 尾节数 —— 人形 = 0 不渲染尾巴 */
  tailSegments: number
  /** 肚皮存在（人形忽略） */
  belly: boolean
  /** 人形专属：躯干(肩→骨盆)高度 */
  torsoLength?: number
  /** 人形专属：腿长(髋→足) */
  legLength?: number
  /** 人形专属：手臂(肩→手)长度 */
  armLength?: number
  /** 人形专属：整体身高(足→头顶)缩放；<1=头身比更大，更Q */
  heightScale?: number
}

/** 建模分支：beast=球形动物（云绒猫/旧精灵） / humanoid=Q版少女（雏菊精灵 v2） */
export type ModelKind = 'beast' | 'humanoid'

/** 单部件位姿片断（tween 目标；缺省字段 = 不动） */
export interface BonePosePart {
  /** 绕 Z 旋转（弧度） */
  rotZ?: number
  /** 绕 X 旋转（弧度） */
  rotX?: number
  /** 本地 Y 位移 */
  posY?: number
  /** 统一缩放 */
  scale?: number
  /** 仅 Y 缩放（眯眼用） */
  scaleY?: number
  /** 透明度（0..1，缺省保持） */
  opacity?: number
}

/** 一种心情下全体可动骨骼的目标位姿 */
export interface EmotionPose {
  body: BonePosePart
  head: BonePosePart
  leftEar: BonePosePart
  rightEar: BonePosePart
  leftEye: BonePosePart
  rightEye: BonePosePart
  leftBrow: BonePosePart
  rightBrow: BonePosePart
  mouth: {
    /** svg/网格表情形态：smile / open / frown / swoon / none */
    shape: 'smile' | 'open' | 'frown' | 'swoon' | 'none'
    /** 嘴打开幅度（0..1） */
    open?: number
  }
  blush: {
    /** 腮红透明度（0..1） */
    opacity: number
  }
  /** 瞳孔放大（专注/兴奋）；1 = 常态 */
  pupilScale: number
}

/** 动画参数（频率 Hz / 振幅）—— 皮肤可覆盖 */
export interface MotionSpec {
  breath: { freq: number; amp: number }
  float: { freq: number; amp: number }
  blink: { baseSeconds: number; jitterSeconds: number; doubleBlinkProb: number }
  tail: { freq: number; amp: number }
  wing: { freq: number; amp: number }
  earTwitch: { everySeconds: number; amp: number }
  haloSpin: number
  /** happy 蹦跳：进入开心心情时周期性小跳（freq=跳频 Hz / amp=跳高） */
  bounce: { freq: number; amp: number }
  /** sleepy 揉眼：打盹时手臂上抬揉眼（armLiftBase=基础抬升 / armLiftAmp=揉动幅度 / eyeSquint=眯眼系数） */
  rub: { armLiftBase: number; armLiftAmp: number; eyeSquint: number }
  /** 部位点击反应：point 点击后 elapsed 秒的衰减冲量
   *  （decay=指数衰减率 / freq=摆动频率 Hz / headAmp·backAmp·tailAmp=头/背/尾幅度） */
  poke: { decay: number; freq: number; headAmp: number; backAmp: number; tailAmp: number }
}

/** 完整模型规格（一个可热插拔的「桌宠皮肤 = 3D 模型」） */
export interface PetModelSpec {
  id: string
  label: string
  /** 主题色（翅膀/光环/轮廓光），0xRRGGBB */
  accent: number
  /** 建模分支（必填；v2 新增，默认 beast 向后兼容） */
  kind: ModelKind
  /** 外观差异化（皮肤专属：发色/耳型/花型头饰/尾型/衣裙）—— 缺省按 DEFAULT_APPEARANCE */
  appearance?: Partial<AppearanceSpec>
  /** 建模几何 */
  proportions: ProportionsSpec
  /** 每种 3D 心情的表情目标位姿（必须覆盖全部 PetMood3D） */
  emotionPoses: Record<PetMood3D, EmotionPose>
  /** 动画参数 */
  motion: MotionSpec
}

/** 外观差异化（精灵 vs 猫）—— 数据驱动，渲染器按此变化造型 */
export interface AppearanceSpec {
  /** 主毛色/发色 */
  furColor: number
  /** 次级毛色（尾尖/影色/发梢） */
  furAccent: number
  /** 肤色（人形必备；兽形忽略） */
  skin: number
  /** 主衣裙色（人形必备；兽形忽略） */
  dressColor: number
  /** 衣裙点缀色（花瓣裙的外层薄纱色） */
  dressAccent: number
  /** 鞋色（人形芭蕾鞋/兽形爪垫） */
  shoeColor: number
  /** 耳型：猫耳（锥体） / 精灵细长尖耳 */
  earStyle: 'cat' | 'elf'
  /** 花型头饰（桌面宠头顶装饰的颜色形态） */
  ornament: 'daisy' | 'blossom' | 'lavender' | 'clover' | 'none'
  /** 尾型：猫球尾 / 精灵细长流光尾 / 人形 none */
  tailStyle: 'puff' | 'wisp' | 'none'
  /** 人形专属：发型（long_wave=超长波浪 / twin_tails=双尾 / straight=直发） */
  hairStyle: 'long_wave' | 'twin_tails' | 'straight'
  /** 人形专属：裙型（petal_dress=花瓣嫁裙 / frill=荷叶裙 / tunic=束腰长衣） */
  dressStyle: 'petal_dress' | 'frill' | 'tunic'
  /** 人形专属：翼型（butterfly=蝶翼4片 / fairy=双蝉翼 / none） */
  wingStyle: 'butterfly' | 'fairy' | 'none'
  /** 人形专属：持花束（捧花）是否显示 */
  hasBouquet: boolean
}

/** 默认外观（云绒猫）—— 兽形基准，人形字段给安全默认值 */
export const DEFAULT_APPEARANCE: AppearanceSpec = {
  furColor: 0xf6f7fb,
  furAccent: 0xc9d6ea,
  skin: 0xfff4ed,
  dressColor: 0xffffff,
  dressAccent: 0xcfe3f3,
  shoeColor: 0xeef4fb,
  earStyle: 'cat',
  ornament: 'daisy',
  tailStyle: 'puff',
  hairStyle: 'long_wave',
  dressStyle: 'petal_dress',
  wingStyle: 'butterfly',
  hasBouquet: true,
}

/** 外观字段默认值合并（旧 Spec 无 appearance 时自动补齐） */
export function mergeAppearance(spec: PetModelSpec): AppearanceSpec {
  return { ...DEFAULT_APPEARANCE, ...(spec.appearance ?? {}) }
}

/**
 * 眼部锚点（虹膜/瞳孔与高光在头部本地坐标的基准位）。
 *
 * **单一事实源**：构建器（摆放）与动画器（`applyPose` 逐帧覆写坐标）必须共用同一份锚点。
 * 此前 `buildPetHumanoid` 按 ±0.16/0.03 布局，而 `applyPose` 用兽形的 ±0.18/0.04 覆写
 * 瞳孔与高光坐标 —— 人形的虹膜与高光因此相对眼白错位（真实可见缺陷）。
 * 收敛到本表后，该缺陷在结构上不可能复现。
 */
export interface EyeAnchors {
  /** 左/右眼白基准坐标 */
  eyeL: { x: number; y: number }
  eyeR: { x: number; y: number }
  /** 左/右虹膜（人形）或瞳孔（兽形）基准坐标；X 为负 = 左 */
  pupilL: { x: number; y: number }
  pupilR: { x: number; y: number }
  /** 左/右高光基准坐标 */
  shineL: { x: number; y: number }
  shineR: { x: number; y: number }
}

/** 兽形（猫/精灵）眼位：与旧版 Pet3D 视觉等值 */
export const BEAST_EYE_ANCHORS: EyeAnchors = {
  eyeL: { x: -0.18, y: 0.05 },
  eyeR: { x: 0.18, y: 0.05 },
  pupilL: { x: -0.18, y: 0.04 },
  pupilR: { x: 0.18, y: 0.04 },
  shineL: { x: -0.15, y: 0.095 },
  shineR: { x: 0.21, y: 0.095 },
}

/** 人形眼位：大而圆的大眼，眼距略窄于兽形 */
export const HUMANOID_EYE_ANCHORS: EyeAnchors = {
  eyeL: { x: -0.16, y: 0.03 },
  eyeR: { x: 0.16, y: 0.03 },
  pupilL: { x: -0.16, y: 0.03 },
  pupilR: { x: 0.16, y: 0.03 },
  shineL: { x: -0.135, y: 0.075 },
  shineR: { x: 0.185, y: 0.075 },
}

/** 按形态取眼部锚点（构建与动画共用的唯一入口） */
export function eyeAnchorsFor(kind: ModelKind): EyeAnchors {
  return kind === 'humanoid' ? HUMANOID_EYE_ANCHORS : BEAST_EYE_ANCHORS
}

/**
 * 人形发丝的垂直布局（**相对头部的本地坐标**，头部中心 y = 0，头顶 = +headRadius）。
 *
 * 单一事实源 + 回归守卫：鬓发/飘丝的 Y 此前被写成「接近 1.0 的绝对高度」，
 * 但父节点是位于 y = 1.5 的头部 —— 结果 6 缕鬓发整段飘在**头顶之上**
 * （实测世界 y 2.15~2.41，而头顶仅 1.98），与代码注释「顺肩而下」完全相反。
 * 现统一为「自头部上缘向下垂落」的本地坐标，并由单测锁定不得高过头顶。
 */
export const HUMANOID_HAIR = {
  /** 鬓发起始高度：落在头部上缘之内（headRadius ≈ 0.48），保证不飘出头顶 */
  sideLockTopY: 0.15,
  /** 每缕鬓发相对上一缕的下移量（越靠后越短、越往下） */
  sideLockStepY: 0.03,
  /** 飘丝长度（细长装饰丝） */
  wispLength: 0.45,
  /** 飘丝基准高度：环绕头部前方飘动，仍在头顶之下 */
  wispY: 0.22,
} as const

/** 一缕发丝的本地摆放（构建器直接消费；纯数据，可单测） */
export interface HairStrandPlacement {
  x: number
  y: number
  z: number
  /** 胶囊/圆柱长度（沿本地 Y） */
  length: number
  /** 发丝粗细（胶囊半径） */
  radius: number
  rotationZ: number
  rotationX: number
}

/**
 * 两侧鬓发（顺肩而下）：中心 = 起始高度 − 半长，
 * 因此整缕都在头部中心之下，向下垂落到肩部。
 */
export function humanoidSideLocks(): HairStrandPlacement[] {
  const out: HairStrandPlacement[] = []
  for (const side of [-1, 1] as const) {
    for (let k = 0; k < 3; k++) {
      const length = 1.0 - k * 0.12
      out.push({
        x: side * (0.28 + k * 0.04),
        y: HUMANOID_HAIR.sideLockTopY - length * 0.5 - k * HUMANOID_HAIR.sideLockStepY,
        z: 0.02,
        length,
        radius: 0.085 - k * 0.012,
        rotationZ: side * (0.1 + k * 0.08),
        rotationX: -0.1,
      })
    }
  }
  return out
}

/** 几缕细长飘丝（环绕头部前方飘动装饰） */
export function humanoidWisps(): HairStrandPlacement[] {
  const out: HairStrandPlacement[] = []
  for (let k = 0; k < 5; k++) {
    const ang = (k - 2) * 0.45
    out.push({
      x: Math.sin(ang) * 0.44,
      y: HUMANOID_HAIR.wispY,
      z: Math.cos(ang) * 0.3,
      length: HUMANOID_HAIR.wispLength,
      radius: 0.008,
      rotationZ: Math.sin(ang) * 0.3,
      rotationX: -0.2,
    })
  }
  return out
}

/**
 * 卡通三段渐变档位（LOW / MID / HIGH）—— `MeshToonMaterial.gradientMap` 的取样值。
 *
 * **为什么会灰**：`MeshToonMaterial` 把基色**乘以**渐变取样值。此前 LOW 档写死
 * `0x7a7a7a`（仅 48% 亮度），于是纯白内裙（0xffffff）在背光面渲染成 48% 灰、
 * 近白肤色（0xffede0）渲染成灰褐——整只宠物看起来「灰蓝、发脏」，
 * 而材质取色其实完全正确（已用运行时探测确认）。
 *
 * ⚠️ **R59 真正的根因（比「LOW 太暗」更根本）**：three 的 `getGradientIrradiance` 取样坐标是
 * ```
 *   vec2 coord = vec2( dotNL * 0.5 + 0.5, 0.0 );   // three r185 lights_toon_pars_fragment
 *   vec3 irradiance = getGradientIrradiance(...) * directLight.color;  // RE_Direct_Toon
 * ```
 * 且 `dotNL` 在光照循环里已被 `saturate` —— 于是**受光面（dotNL ∈ [0,1]）只可能采样到
 * coord.x ∈ [0.5, 1] 这一段**。旧贴图是 3 texel，最近邻下受光面只能取到 texel#1/#2，
 * 即 `0xee(0.933)` 与 `0xff(1.0)`：**LOW 档（0xc6）对受光面是死代码**。
 * 这解释了历次实验里两个费解现象：①白色部件平坦无立体感（可见明暗差只有 6.7%）
 * ②「改 LOW 档位 → 采样像素 Δ=0」（受光面根本采不到那一格）。
 *
 * 注意 `RE_Direct_Toon` 里**没有独立的 N·L 因子** —— 渐变取样值**替代**了 N·L，这是卡通着色的定义。
 *
 * ⇒ 修法：让档位落在**可达的那半段**。贴图宽度取 6、可见半段即 texel#3/#4/#5，
 * 对应 dotNL 的三个区间 [0,1/3) / [1/3,2/3) / [2/3,1]（`PET_TOON_RAMP_WIDTH` 锁死）。
 * LOW 因此第一次真正出现在受光面上，且仍 ≥ `PET_TOON_LOW_FLOOR`（不显脏）。
 */
export const PET_TOON_RAMP = {
  /** 背光/掠射面（dotNL ∈ [0, 1/3)）：柔和压深，绝不压到中灰 */
  low: 0xc6c6c6,
  /** 中间调（dotNL ∈ [1/3, 2/3)） */
  mid: 0xdcdcdc,
  /** 受光面高光（dotNL ∈ [2/3, 1]） */
  high: 0xffffff,
} as const

/**
 * 渐变贴图 texel 数。**必须是偶数**：受光面只会采样到后半段
 * （`coord.x = dotNL*0.5+0.5 ∈ [0.5,1]`），前半段是背光面的死区。
 */
export const PET_TOON_RAMP_WIDTH = 6

/** 渐变档位下限（回归守卫：低于此值会让白色部件显脏） */
export const PET_TOON_LOW_FLOOR = 0xb0

/**
 * 渐变贴图的 **RGBA 字节布局**（`PET_TOON_RAMP_WIDTH` texel × 4 字节，alpha 恒 255）。
 *
 * 布局是 `[LOW×4, MID, HIGH]`：texel#0..#2 是背光死区，texel#3 起才是受光面可达的三档
 * （见 `PET_TOON_RAMP` 的推导）。
 *
 * 为什么要单独抽出来并加守卫：`THREE.RGBFormat` 在 three r152+ 已被移除。用 RGBFormat +
 * 每 texel 3 字节会让贴图不完整、**取样恒为 0**，于是所有 `MeshToonMaterial` 的
 * 平行光/点光贡献全部归零（只剩半球光），宠物呈平坦发灰——该缺陷已由运行时实验定位。
 */
export function toonRampBytes(): number[] {
  // texel#0..#(WIDTH/2-1) 是背光死区（受光面采样不到），其后才是可达的 LOW → MID → HIGH。
  // 注意可达段的第一个 texel 也是 LOW：dotNL=0 时 coord.x=0.5 正好落在它上面。
  const reachable = [PET_TOON_RAMP.low, PET_TOON_RAMP.low, PET_TOON_RAMP.mid, PET_TOON_RAMP.high]
  const deadZone = Array<number>(PET_TOON_RAMP_WIDTH - reachable.length).fill(PET_TOON_RAMP.low)
  const out: number[] = []
  for (const c of [...deadZone, ...reachable]) {
    out.push((c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff, 0xff)
  }
  return out
}

/**
 * 复刻 three `getGradientIrradiance` 的取样：给定 `dotNL` 返回渐变档位（0..1 乘数）。
 *
 * 唯一事实源是上面的贴图布局 —— 守卫与运行时必须用同一份推导，否则又会漂移。
 * `coord.x = dotNL*0.5 + 0.5`（受光面 → [0.5,1]），texel 索引 = `floor(coord.x × WIDTH)`。
 */
export function toonBandFor(dotNL: number): number {
  const clamped = Math.min(1, Math.max(0, dotNL))
  const coord = clamped * 0.5 + 0.5
  const index = Math.min(
    PET_TOON_RAMP_WIDTH - 1,
    Math.max(0, Math.floor(coord * PET_TOON_RAMP_WIDTH)),
  )
  // 可达段 = [WIDTH/2, WIDTH-1]，映射到 [LOW, MID, HIGH]
  const reachable = [PET_TOON_RAMP.low, PET_TOON_RAMP.mid, PET_TOON_RAMP.high]
  const offset = PET_TOON_RAMP_WIDTH - reachable.length
  const level = reachable[Math.min(reachable.length - 1, Math.max(0, index - offset))]
  return ((level >> 16) & 0xff) / 255
}

/* ── 灯光规格（数据驱动 + 过曝守卫）────────────────────────────────────────
 *
 * **为什么必须收口成数据**：渐变贴图修好（RGBFormat → RGBAFormat）后，平行光/点光贡献
 * 从「全部归零」恢复到真实值，原本为「补偿不亮」而堆高的强度立刻**过曝**——
 * 运行时投影采样实测：躯干基色 0xffffff 出图 0xffffff、肤色 0xffede0 出图也是 0xffffff。
 * 更关键的是**渐变三段被一起裁到 1.0**：卡通明暗分界彻底消失，白裙变成没有体积感的死白。
 *
 * **正确的配平口径**（依据 three 的真实着色链，不是拍脑袋）：
 *   `RE_Direct_Toon`:  irradiance = gradient(N·L) × lightColor × intensity
 *   `BRDF_Lambert`  :  RECIPROCAL_PI × diffuseColor        ← 即**除以 π**
 * 故纯白表面（linear albedo = 1）的最终线性亮度 = (ambient + Σ direct×档位) / π。
 * 目标：**HIGH 段 ≈ 1.0（受光面到纯白）、LOW 段 ≈ 0.78（背光面仍是亮部）**，
 * 两者相差 ~0.21，卡通分界清晰可见。总量因此配在 π 附近，而不是 1 附近。
 */
export interface PetLightingSpec {
  /** 半球光：天空色 / 地面反弹色 / 强度（**不做渐变量化**，线性参与） */
  hemi: { sky: number; ground: number; intensity: number }
  /** 主光：方向由 position 指向原点，贡献经渐变贴图量化 */
  key: { color: number; intensity: number; position: readonly [number, number, number] }
  /** 补光：压低背光面，避免死黑 */
  fill: { color: number; intensity: number; position: readonly [number, number, number] }
  /** 轮廓光：用主题色勾边（点光源，受距离衰减） */
  rim: { intensity: number; distance: number; position: readonly [number, number, number] }
  /** 底部反弹光（点光源） */
  bottom: { color: number; intensity: number; distance: number; position: readonly [number, number, number] }
  /** 正面微补光（仅人形；保证脸部最小亮度过渡） */
  front?: { color: number; intensity: number; position: readonly [number, number, number] }
}

/**
 * 两形态灯光配平表。数值按上方的 π 口径解出。
 *
 * ⚠️ **R59 更正（重要）**：此前这里引用的「运行时分组实测」数字（hemi 0.270 / dir 0.898 / …）
 * 来自**采样点落在背景或被遮挡网格上**的无效实验（见《3D WebGL Agent Workspace》最后两轮：
 * 两次自我推翻，结论是「采样方法本身不可靠」）。R59 改用**可验证的隔离法**重测：
 * 元素裁剪截图 + 把画布 `visibility:hidden` 再截一张，取两次之差（两次独立试验 IoU ≥ 0.97 才采信），
 * 于是 alpha/背景污染与遮挡都不再可能。
 *
 * 实测（R59，隔离网关 + Playwright，dpr=1，overlay 路由，5 款皮肤逐一体测；
 * 隔离法 = 隐藏画布前后差集 + 两次试验互证 IoU ≥ 0.93）：
 *
 *   皮肤（kind）        修前 中位/≥235/线性      修后 中位/≥235/线性
 *   橘猫小工（beast）    255 / 75.4% / 1.000  →  221 / 30.9% / 0.72
 *   雏菊精灵（humanoid） 229 / 42.4% / 0.788  →  220 / 37.3% / 0.72
 *   樱花精灵（humanoid） 221 / 35.9% / 0.727  →  213 / 27.6% / 0.669
 *   薰衣草精灵（humanoid）202 / 34.3% / 0.596 →  195 / 24.9% / 0.55
 *   四叶草精灵（humanoid）228 / 34.6% / 0.781 →  217 / 28.6% / 0.702
 *
 * 逐灯分解（同一隔离法，只让一盏灯进场景；`none` → 全黑 0.000，即遮罩有效性校验通过）：
 *   hemi 0.079 / **key 0.914** / fill 0.162 / rim 0.009 / bottom 0.045（线性中位）
 * 即：旧兽形 key（3.30）单盏就把白色部件推到 0.914，叠加其它灯必然裁平（卡通口径下主光正对面 = 1.371）；
 * 而人形 key（2.08）本就在预算内 —— 两者相差 59%，这是「橘猫像白团」的直接原因。
 * 故兽形 key 3.30 → 1.90（主光正对面 = 0.968 < 1，见 `keyFaceTotal` 守卫）。
 */
export const PET_LIGHTING: Record<ModelKind, PetLightingSpec> = {
  humanoid: {
    hemi: { sky: 0xffffff, ground: 0xeef2f8, intensity: 0.25 },
    key: { color: 0xffffff, intensity: 2.08, position: [0.55, 3.2, 4.2] },
    fill: { color: 0xcfe0f2, intensity: 1.05, position: [-2, 3.0, 2] },
    rim: { intensity: 0.9, distance: 12, position: [0, 2.2, -1.8] },
    bottom: { color: 0xffffff, intensity: 0.29, distance: 6, position: [0, -0.5, 0.6] },
    front: { color: 0xffffff, intensity: 0.31, position: [0, 0.8, 3.0] },
  },
  beast: {
    hemi: { sky: 0xffffff, ground: 0xdde4f0, intensity: 0.30 },
    // R59：3.30 → 1.90。旧值在**卡通口径**下让几乎整个可见半球的总量 >1（实测 ≥235 占比 75.4%、
    // 线性中位 1.000，白色绒毛整只裁平）；1.90 使「主光正对面」的上界 = 0.968 < 1（见 G4 守卫）。
    key: { color: 0xffffff, intensity: 1.90, position: [2, 4, 3] },
    fill: { color: 0xb8d4f0, intensity: 0.88, position: [-2, 2, 2] },
    rim: { intensity: 0.55, distance: 8, position: [0, 1.6, -1.8] },
    bottom: { color: 0xffffff, intensity: 0.15, distance: 4, position: [0, -0.5, 0.6] },
  },
}

/**
 * 相机框景规格（每个建模分支一套）—— 渲染器不得写死 FOV / 机位 / 视点。
 *
 * 此前 `Pet3D` 构造器里按 `spec.kind === 'humanoid'` 直接写 `new PerspectiveCamera(40, …)`
 * 与 `position.set(0, 1.18, 5.2)`：换皮肤只能改几何、改不了取景，情绪也影响不到构图。
 * 收敛到本表后：①FOV/机位是数据（新皮肤可换取景）②情绪与悬停对**同一个注视点**做推近/拉远。
 */
export interface PetCameraSpec {
  /** 垂直视野（度） */
  fov: number
  /** 机位（世界坐标） */
  position: readonly [number, number, number]
  /**
   * 注视点（世界坐标）—— 一切推近/拉远都绕它做，
   * 于是构图中心永不漂移（推近只是「走近两步」，不是转头）。
   */
  lookAt: readonly [number, number, number]
}

export const PET_CAMERA: Record<ModelKind, PetCameraSpec> = {
  // 人形：整体比兽形高 ~1.9，镜头在胸口高度、看向脸部中心（脸在头 y=1.5 的前 +Z 方向）
  humanoid: { fov: 40, position: [0, 1.18, 5.2], lookAt: [0, 1.30, 0] },
  // 兽形：云绒猫重心低，镜头贴近一点看眼睛
  beast: { fov: 35, position: [0, 0.7, 4.6], lookAt: [0, 0.3, 0] },
}

/**
 * 情绪推近/拉远系数（>1 推近，1 = 基准构图）。**必须覆盖全部 6 种心情**（新增心情即编译期报错）。
 *
 * 取值依据（与 `emotionPoses` 的表现力对齐，绝不虚构）：
 * - excited / happy：凑近看 —— 表情幅度最大，值得占满画面
 * - working / normal：保持基准 —— 用户正在盯它做事，镜头晃会显得毛躁
 * - sad / sleepy：退开一点 —— 「给它空间」是情绪表达，也让「安静」在构图上成立
 */
export const PET_EMOTION_ZOOM: Record<PetMood3D, number> = {
  excited: 1.1,
  happy: 1.05,
  normal: 1,
  working: 1,
  sad: 0.97,
  sleepy: 0.93,
}

/** 悬停聚焦推近系数（>1 推近）：光标停在它身上 = 「我在看你」 */
export const PET_HOVER_ZOOM = 1.06

/** 推近/拉远的逼近速率（1/s，指数逼近；越大越跟手，越小越"沉稳"） */
export const PET_CAMERA_ZOOM_RATE = 3.2

/** 悬停聚焦的注视增益（作用于 petPose 的视线/瞳孔，数据驱动） */
export const PET_HOVER_FOCUS = {
  /** 头/视线转动幅度放大系数（× focus） */
  gazeGain: 0.45,
  /** 瞳孔位移放大系数（× focus） */
  pupilGain: 0.5,
  /** 瞳孔放大系数（× focus；「盯着你」的生理反应） */
  pupilScaleGain: 0.12,
} as const

/** 纯函数：心情 + 悬停权重(0..1) → 目标推近系数（异常输入当 0 处理，绝不放大成过近） */
export function cameraZoomTarget(mood: PetMood3D, hoverWeight = 0): number {
  const moodZoom = PET_EMOTION_ZOOM[mood] ?? 1
  const hover = Number.isFinite(hoverWeight) ? Math.min(1, Math.max(0, hoverWeight)) : 0
  // 悬停增益按权重**线性叠加**在情绪基线上：两项相乘会叠成过度贴近（1.1×1.06 尚可，
  // 但若日后调高任一常数就会突然怼脸），线性叠加的边界始终可控。
  return moodZoom * (1 + (PET_HOVER_ZOOM - 1) * hover)
}

/** 纯函数：指数逼近（帧率无关）—— dt 秒内向 target 逼近，永不越过 */
export function stepCameraZoom(
  current: number,
  target: number,
  dt: number,
  rate = PET_CAMERA_ZOOM_RATE,
): number {
  if (!Number.isFinite(current) || !Number.isFinite(target)) return target
  const k = 1 - Math.exp(-Math.max(0, dt) * Math.max(0, rate))
  const next = current + (target - current) * k
  // 浮点收尾：差距小于千分之一就直接落到目标（避免永远差一点点）
  return Math.abs(target - next) < 1e-3 ? target : next
}

/** 纯函数：绕注视点推近/拉远后的机位（zoom > 1 更近；非法值退化为基准机位） */
export function zoomedCameraPosition(cam: PetCameraSpec, zoom: number): [number, number, number] {
  const z = Number.isFinite(zoom) && zoom >= 0.2 ? zoom : 1
  const k = 1 / z
  return [
    cam.lookAt[0] + (cam.position[0] - cam.lookAt[0]) * k,
    cam.lookAt[1] + (cam.position[1] - cam.lookAt[1]) * k,
    cam.lookAt[2] + (cam.position[2] - cam.lookAt[2]) * k,
  ]
}

/** 过曝估算结果（**已含 BRDF 的 1/π**，可直接与 1.0 比较） */
export interface PetExposureEstimate {
  /** 半球光贡献（不经过渐变贴图） */
  ambient: number
  /** 平行光贡献（**已含**各灯自身的卡通档位 —— 见下方口径说明） */
  direct: number
}

/**
 * 估算「法线朝 `normal` 的纯白表面」的最终线性亮度分解，供过曝守卫使用。
 * 返回值已除以 π（对齐 three 的 `BRDF_Lambert`），故 `ambient + direct` 即最终亮度，
 * 与 1.0 直接可比。
 *
 * **卡通口径（R59 更正）**：`MeshToonMaterial` 的直接光是
 * `渐变档位(dotNL) × lightColor × albedo/π` —— **没有独立的 N·L 因子**，档位取样值
 * 就是 N·L 的替代物（three r185 `RE_Direct_Toon`）。旧版本按「intensity × dotNL / π」
 * 估算，与着色器**形式不符**：这正是它算不准、且守卫看不见真问题（白色部件整只裁平）的原因。
 *
 * **诚实声明**：仍只计**平行光 + 半球光**。点光源（rim/bottom）的贡献依赖与网格的距离，
 * 纯函数拿不到几何，故不纳入；其真实影响由运行时隔离实测校准（R59：rim 线性格中位 0.009 /
 * bottom 0.045，但 bottom 在腹部局部可达 ~0.4）。本估算的作用是给
 * 「总量 > 1.0 导致渐变档位被裁平」设一道上界。
 */
export function estimateExposure(
  lighting: PetLightingSpec,
  normal: readonly [number, number, number],
): PetExposureEstimate {
  const [nx, ny, nz] = normal
  const len = Math.hypot(nx, ny, nz) || 1
  const n: [number, number, number] = [nx / len, ny / len, nz / len]
  // HemisphereLight：mix(ground, sky, 0.5 * n·up + 0.5) * intensity
  const ambient = (lighting.hemi.intensity * (0.5 + 0.5 * n[1])) / Math.PI
  let direct = 0
  const dirs = [lighting.key, lighting.fill, lighting.front]
  for (const d of dirs) {
    if (!d) continue
    const [px, py, pz] = d.position
    const dl = Math.hypot(px, py, pz) || 1
    const dot = n[0] * (px / dl) + n[1] * (py / dl) + n[2] * (pz / dl)
    // 光照循环里 dotNL 已被 saturate；卡通口径下直接光 = 档位(dotNL) × 强度 / π
    direct += (d.intensity * toonBandFor(Math.max(0, dot))) / Math.PI
  }
  return { ambient, direct }
}

/** 相机可见半球上的曝光统计（**与截图像素统计同口径**） */
export interface PetVisibleExposure {
  /** 按可见性（投影面积 n·view）加权的亮度中位数；1.0 = 纯白 */
  median: number
  /** 被裁到 1.0 的可见面积占比 —— 「整只裁平」的量化指标 */
  clippedShare: number
}

/**
 * 在**相机可见半球**上统计曝光剖面。
 *
 * 为什么必须这样测：旧守卫只在 `[0,0,1]` 这一个法线方向上取值（见 petSpec.test.ts 的
 * `FRONT`），而兽形 key=3.30 的问题恰恰出在「法线更朝向主光的那些面」——单点采样完全看不见。
 * R59 的运行时实测证实了这个盲区：模型判兽形只有 28.2% 的可见面积裁切，实测像素却有 75.4%
 * 落在 ≥235（线性中位 1.000）；而同表下的人形实测仅 34~42%。方向集是确定性的
 * Fibonacci 球（无随机数，测试可复现），权重取 `n·view`（即投影面积，等价于截图上该法线
 * 覆盖的像素数）。
 */
export function visibleExposureProfile(
  lighting: PetLightingSpec,
  camera: PetCameraSpec,
  samples = 2000,
): PetVisibleExposure {
  const vx = camera.lookAt[0] - camera.position[0]
  const vy = camera.lookAt[1] - camera.position[1]
  const vz = camera.lookAt[2] - camera.position[2]
  const vl = Math.hypot(vx, vy, vz) || 1
  // 相机沿 -view 看出去，所以「朝向相机」的表面法线是 -view
  const fx = -vx / vl
  const fy = -vy / vl
  const fz = -vz / vl
  const golden = Math.PI * (3 - Math.sqrt(5))
  let weight = 0
  let clipped = 0
  const rows: { value: number; w: number }[] = []
  for (let i = 0; i < samples; i += 1) {
    const y = 1 - (i / Math.max(1, samples - 1)) * 2
    const r = Math.sqrt(Math.max(0, 1 - y * y))
    const th = golden * i
    const n: [number, number, number] = [Math.cos(th) * r, y, Math.sin(th) * r]
    const facing = n[0] * fx + n[1] * fy + n[2] * fz
    if (facing <= 0) continue
    const e = estimateExposure(lighting, n)
    // 卡通口径下 direct 已含档位，直接相加即该法线的最终亮度
    const value = e.ambient + e.direct
    weight += facing
    if (value > 1) clipped += facing
    rows.push({ value, w: facing })
  }
  if (weight === 0) return { median: 0, clippedShare: 0 }
  rows.sort((a, b) => a.value - b.value)
  let acc = 0
  let median = rows[rows.length - 1].value
  for (const row of rows) {
    acc += row.w
    if (acc >= weight / 2) {
      median = row.value
      break
    }
  }
  return { median, clippedShare: clipped / weight }
}

/* ── 眼部几何（相对 headRadius 的比例 + 纯函数展开）─────────────────────────
 *
 * **为什么必须相对化**（人形「没有脸」的根因）：此前眼白半径写成绝对常量
 * （人形 `0.17 × eyeScale(1.35) = 0.23`），而 `headRadius` 随皮肤变（兽 0.52 / 人形 0.48）。
 * 于是人形两只眼白球直径 0.46 而左右眼心距仅 0.32 —— **两眼融成一坨白色巨球盖住整张脸**；
 * 同时虹膜 Z 偏移只有 0.002，远小于眼白半径 0.23 —— **虹膜/瞳孔/高光全被眼白吞没**。
 * 两者叠加，运行时截图里人形只剩一颗白球和一条下巴。
 *
 * 改为「相对 headRadius 的比例」后，眼型随头围自动等比缩放；比例值取自**兽形已验证的现值**
 * （0.14/0.52 = 0.269、0.08/0.14 = 0.571…），故猫的视觉零回归。
 */
export const PET_EYE = {
  /** 眼白球半径 / headRadius（= 兽形现值 0.14 / 0.52，精确复现） */
  whiteOfHead: 14 / 52,
  /** 虹膜半径 / 眼白半径（= 兽形现值 0.08 / 0.14） */
  irisOfWhite: 8 / 14,
  /** 瞳孔半径 / 眼白半径（虹膜的 0.58 倍） */
  pupilOfWhite: (8 / 14) * 0.58,
  /** 大高光半径 / 眼白半径（= 兽形现值 0.03 / 0.14） */
  shineOfWhite: 3 / 14,
  /** 小高光半径 / 眼白半径 */
  shineSmallOfWhite: 0.13,
  /** 眼白球心 Z / headRadius（= 兽形现值 0.42 / 0.52） */
  eyeZOfHead: 42 / 52,
  /**
   * 各层沿 +Z 的推距（× 眼白半径）。
   * **硬约束**：`推距 + 本层半径比 > 1`，否则该层被眼白球吞掉（正是人形脸消失的机制），
   * 由 `eyeLayout` 守卫测试锁死。iris 档取兽形现值 (0.52−0.42)/0.14，猫零回归。
   */
  irisPushOfWhite: 10 / 14,
  pupilPushOfWhite: 0.86,
  shinePushOfWhite: 15 / 14,
  shineSmallPushOfWhite: 1.0,
} as const

/** 眼部各层的半径与 Z 深度（头部本地坐标，头球心 = 原点） */
export interface EyeLayout {
  whiteR: number
  irisR: number
  pupilR: number
  shineR: number
  shineSmallR: number
  /** 眼白球心 Z */
  eyeZ: number
  irisZ: number
  pupilZ: number
  shineZ: number
  shineSmallZ: number
}

/** 由头围与放大系数展开眼部几何（纯函数：建模与守卫测试共用同一份算式） */
export function eyeLayout(headRadius: number, eyeScale = 1): EyeLayout {
  const s = Number.isFinite(eyeScale) && eyeScale > 0 ? eyeScale : 1
  const whiteR = headRadius * PET_EYE.whiteOfHead * s
  const eyeZ = headRadius * PET_EYE.eyeZOfHead
  return {
    whiteR,
    irisR: whiteR * PET_EYE.irisOfWhite,
    pupilR: whiteR * PET_EYE.pupilOfWhite,
    shineR: whiteR * PET_EYE.shineOfWhite,
    shineSmallR: whiteR * PET_EYE.shineSmallOfWhite,
    eyeZ,
    irisZ: eyeZ + whiteR * PET_EYE.irisPushOfWhite,
    pupilZ: eyeZ + whiteR * PET_EYE.pupilPushOfWhite,
    shineZ: eyeZ + whiteR * PET_EYE.shinePushOfWhite,
    shineSmallZ: eyeZ + whiteR * PET_EYE.shineSmallPushOfWhite,
  }
}

/* ── 前刘海（额前碎发）几何 ────────────────────────────────────────────────
 *
 * **硬约束：底缘必须高于眼白顶端**。此前 7 片切片中心在 `y=0.12`、半高 `0.145`，
 * 底缘落到 `y=-0.025` —— 比眼白顶端（`0.03 + 0.142 = 0.172`）低了 0.20，
 * 于是运行时截图里**每只眼睛的上半部被齐刘海切掉一条**，看起来像半圆。
 * 由 `fringeBottomY` / `humanoidEyeTopY` 两个纯函数把这条约束变成守卫测试。
 */
export const HUMANOID_FRINGE = {
  /** 切片数（奇数，中央一片对齐眉心） */
  count: 7,
  /** 切片球半径 / headRadius */
  radiusOfHead: 0.177,
  /** 垂直拉伸系数（半高 = 半径 × 该系数） */
  stretchY: 1.0,
  /** 深度压缩系数（半深 = 半径 × 该系数） */
  squashZ: 0.62,
  /** 切片中心高度 / headRadius（底缘 = 该值 − 半高） */
  yOfHead: 0.65,
  /** 相邻切片横向间距 / headRadius */
  pitchOfHead: 0.177,
} as const

/** 刘海底缘高度（头部本地坐标）—— 守卫用：必须高于眼白顶端 */
export function fringeBottomY(headRadius: number): number {
  const half = headRadius * HUMANOID_FRINGE.radiusOfHead * HUMANOID_FRINGE.stretchY
  return headRadius * HUMANOID_FRINGE.yOfHead - half
}

/** 人形眼白顶端高度（头部本地坐标）—— 守卫用：刘海不得低于此值 */
export function humanoidEyeTopY(headRadius: number, eyeScale: number): number {
  return HUMANOID_EYE_ANCHORS.eyeL.y + eyeLayout(headRadius, eyeScale).whiteR
}

/** 各花型头饰的配色（渲染器据此上色；'none' 不显示） */
export const ORNAMENT_COLORS: Record<Exclude<AppearanceSpec['ornament'], 'none'>, { petal: number; center: number }> = {
  daisy: { petal: 0xffffff, center: 0xf4d06f },
  blossom: { petal: 0xf8c8d8, center: 0xf291a8 },
  lavender: { petal: 0xcdb8e8, center: 0xa98ad0 },
  clover: { petal: 0x9ed0a8, center: 0xf4d06f },
}

/** 默认云绒猫规格 —— 与旧版 Pet3D 视觉等值的参数化版本 */
export const DEFAULT_PET_SPEC: PetModelSpec = {
  id: 'cat',
  label: '茉莉 · 云绒猫 3D',
  kind: 'beast',
  accent: 0x9ec5e8,
  proportions: {
    bodyRadius: 0.58,
    bodyScaleY: 0.88,
    headRadius: 0.52,
    earHeight: 0.32,
    eyeScale: 1.0,
    tailSegments: 3,
    belly: true,
  },
  emotionPoses: {
    normal: {
      body: {},
      head: {},
      leftEar: { rotZ: 0.25 },
      rightEar: { rotZ: -0.25 },
      leftEye: {},
      rightEye: {},
      leftBrow: { rotZ: 0.15, posY: 0.22 },
      rightBrow: { rotZ: -0.15, posY: 0.22 },
      mouth: { shape: 'smile' },
      blush: { opacity: 0.5 },
      pupilScale: 1,
    },
    happy: {
      body: { scale: 1.03 },
      head: { rotZ: 0.14 },
      leftEar: { rotZ: 0.1, posY: 0.02 },
      rightEar: { rotZ: -0.1, posY: 0.02 },
      leftEye: { scaleY: 0.3 },
      rightEye: { scaleY: 0.3 },
      leftBrow: { rotZ: 0.3, posY: 0.26 },
      rightBrow: { rotZ: -0.3, posY: 0.26 },
      mouth: { shape: 'open', open: 0.7 },
      blush: { opacity: 0.85 },
      pupilScale: 1.1,
    },
    sleepy: {
      body: { scale: 0.96, posY: -0.04 },
      head: {},
      leftEar: { rotZ: 0.8, posY: -0.04 },
      rightEar: { rotZ: -0.8, posY: -0.04 },
      leftEye: { scaleY: 0.08 },
      rightEye: { scaleY: 0.08 },
      leftBrow: { rotZ: 0.05, posY: 0.2 },
      rightBrow: { rotZ: -0.05, posY: 0.2 },
      mouth: { shape: 'swoon' },
      blush: { opacity: 0.25 },
      pupilScale: 0.8,
    },
    working: {
      body: { posY: 0.02 },
      head: { rotX: 0.16 },
      leftEar: { rotZ: 0.25 },
      rightEar: { rotZ: -0.25 },
      leftEye: {},
      rightEye: {},
      leftBrow: { rotZ: 0.22, posY: 0.24 },
      rightBrow: { rotZ: -0.22, posY: 0.24 },
      mouth: { shape: 'smile', open: 0.4 },
      blush: { opacity: 0.35 },
      pupilScale: 1.25,
    },
    excited: {
      body: { scale: 1.06 },
      head: { rotZ: 0.1 },
      leftEar: { rotZ: 0.05, posY: 0.04 },
      rightEar: { rotZ: -0.05, posY: 0.04 },
      leftEye: { scaleY: 0.3 },
      rightEye: { scaleY: 0.3 },
      leftBrow: { rotZ: 0.38, posY: 0.28 },
      rightBrow: { rotZ: -0.38, posY: 0.28 },
      mouth: { shape: 'open', open: 1 },
      blush: { opacity: 0.9 },
      pupilScale: 1.3,
    },
    sad: {
      body: { scale: 0.9, posY: -0.06 },
      head: { rotX: 0.24 },
      leftEar: { rotZ: 0.65, posY: -0.03 },
      rightEar: { rotZ: -0.65, posY: -0.03 },
      leftEye: { rotZ: 0.35 },
      rightEye: { rotZ: -0.35 },
      leftBrow: { rotZ: -0.2, posY: 0.17 },
      rightBrow: { rotZ: 0.2, posY: 0.17 },
      mouth: { shape: 'frown' },
      blush: { opacity: 0.2 },
      pupilScale: 0.85,
    },
  },
  motion: {
    breath: { freq: 1.8, amp: 0.03 },
    float: { freq: 1.4, amp: 0.035 },
    blink: { baseSeconds: 2, jitterSeconds: 4, doubleBlinkProb: 0.18 },
    tail: { freq: 3.2, amp: 0.28 },
    wing: { freq: 6, amp: 0.32 },
    earTwitch: { everySeconds: 3, amp: 0.22 },
    haloSpin: 1.6,
    bounce: { freq: 3.1, amp: 0.15 },
    rub: { armLiftBase: 0.6, armLiftAmp: 0.12, eyeSquint: 0.55 },
    poke: { decay: 6.5, freq: 18, headAmp: 0.35, backAmp: 0.22, tailAmp: 0.9 },
  },
}

// ─────────────────────────────────────────────────────────────────────────────
// 精灵皮肤 Spec（4 款：雏菊 / 樱花 / 薰衣草 / 四叶草）
// v2：完全对应用户给的少女精灵立绘——长发/花瓣嫁裙/4片蝶翼/花嫁花环/芭蕾鞋+花束。
//      全部为数据，渲染器零手工；新增皮肤 = 追加 Spec。
// ─────────────────────────────────────────────────────────────────────────────

/** 精灵表情（6 心情全覆盖，兽形人形共用同一套眉毛/嘴/腮红定义——只改几何） */
const ELF_POSES = DEFAULT_PET_SPEC.emotionPoses

/** Q 版少女身体比例：2.2 头身。(头=半径 0.48 球 / 身高约 2.1) */
const HUMANOID_PROPS: ProportionsSpec = {
  bodyRadius: 0.22,
  bodyScaleY: 1,
  headRadius: 0.48,
  earHeight: 0,
  // 1.10（原 1.35）：眼白半径 = headRadius × PET_EYE.whiteOfHead × eyeScale = 0.142。
  // 1.35 会得到 0.174，而左右眼心距仅 0.32 → **两眼白球重叠 0.03、融成一坨盖住整张脸**
  // （运行时截图已确认）。1.10 留出 0.036 的分离量，既是大眼又不成「独眼」。
  eyeScale: 1.10,
  tailSegments: 0,
  belly: false,
  torsoLength: 0.34,
  legLength: 0.58,
  armLength: 0.42,
  heightScale: 1,
}

/** 精灵少女通用 motion：漂浮感更强、裙摆轻摇、蝶翼拍翅 + 发梢飘动相位 */
function girlMotion(): MotionSpec {
  return {
    ...DEFAULT_PET_SPEC.motion,
    float: { freq: 1.2, amp: 0.06 },
    breath: { freq: 1.6, amp: 0.025 },
    wing: { freq: 5.5, amp: 0.42 },
    tail: { freq: 2.2, amp: 0.12 }, // 裙摆代替尾巴使用
    blink: { baseSeconds: 2.6, jitterSeconds: 4.2, doubleBlinkProb: 0.22 },
    earTwitch: { everySeconds: 99, amp: 0 }, // 人形无耳
  }
}

/**
 * 花嫁花环：12 片花瓣环 + 中心主花 + 3 片大叶点缀（完全对应雏菊精灵图片）
 * ornament 颜色随精灵皮肤 id 切换；花束共享 ORNAMENT_COLORS 配色。
 */

/** 精灵 4 款皮肤（accent 与花型/发色/裙色区分） */
export const ELF_SPECS: Record<string, PetModelSpec> = {
  'blue': {
    id: 'blue',
    label: '雏菊精灵 3D · 少女花嫁',
    kind: 'humanoid',
    accent: 0x8ec7ef,
    appearance: {
      // 发色：主=奶白，发梢=冰蓝（对应长发渐变）
      furColor: 0xf4f8fc, furAccent: 0xa6ccee,
      skin: 0xffede0,
      // 花嫁裙：主=奶白，外层薄纱=冰蓝
      dressColor: 0xffffff, dressAccent: 0xb9dcf3,
      shoeColor: 0xe4eef8,
      earStyle: 'elf', ornament: 'daisy', tailStyle: 'none',
      hairStyle: 'long_wave',
      dressStyle: 'petal_dress',
      wingStyle: 'butterfly',
      hasBouquet: true,
    },
    proportions: HUMANOID_PROPS,
    emotionPoses: ELF_POSES,
    motion: girlMotion(),
  },
  'pink': {
    id: 'pink',
    label: '樱花精灵 3D · 少女花嫁',
    kind: 'humanoid',
    accent: 0xf4a8c0,
    appearance: {
      furColor: 0xfdf0f4, furAccent: 0xf5b4cc,
      skin: 0xffede0,
      dressColor: 0xfff4f7, dressAccent: 0xf5c4d6,
      shoeColor: 0xf9e2ea,
      earStyle: 'elf', ornament: 'blossom', tailStyle: 'none',
      hairStyle: 'long_wave',
      dressStyle: 'petal_dress',
      wingStyle: 'butterfly',
      hasBouquet: true,
    },
    proportions: HUMANOID_PROPS,
    emotionPoses: ELF_POSES,
    motion: girlMotion(),
  },
  'purple': {
    id: 'purple',
    label: '薰衣草精灵 3D · 少女花嫁',
    kind: 'humanoid',
    accent: 0xb8a0d8,
    appearance: {
      furColor: 0xf3eef9, furAccent: 0xd0bce8,
      skin: 0xffede0,
      dressColor: 0xfaf4ff, dressAccent: 0xc9b2e8,
      shoeColor: 0xe7def3,
      earStyle: 'elf', ornament: 'lavender', tailStyle: 'none',
      hairStyle: 'long_wave',
      dressStyle: 'petal_dress',
      wingStyle: 'butterfly',
      hasBouquet: true,
    },
    proportions: HUMANOID_PROPS,
    emotionPoses: ELF_POSES,
    motion: girlMotion(),
  },
  'green': {
    id: 'green',
    label: '四叶草精灵 3D · 少女花嫁',
    kind: 'humanoid',
    accent: 0x8ec9a0,
    appearance: {
      furColor: 0xeff7f0, furAccent: 0xbfe0c9,
      skin: 0xffede0,
      dressColor: 0xf3fbf4, dressAccent: 0xc0decc,
      shoeColor: 0xe2f1e6,
      earStyle: 'elf', ornament: 'clover', tailStyle: 'none',
      hairStyle: 'long_wave',
      dressStyle: 'petal_dress',
      wingStyle: 'butterfly',
      hasBouquet: true,
    },
    proportions: HUMANOID_PROPS,
    emotionPoses: ELF_POSES,
    motion: girlMotion(),
  },
}

/**
 * 按皮肤 id 取模型 Spec（data-driven 热插拔入口）。
 * - 猫 'cat' → DEFAULT_PET_SPEC
 * - 精灵 → ELF_SPECS 对应项
 * - 未知 id → 猫（兜底，绝不返回 undefined）
 */
export function getPetModelSpec(skinId: string): PetModelSpec {
  if (skinId === 'cat') return DEFAULT_PET_SPEC
  return ELF_SPECS[skinId] ?? DEFAULT_PET_SPEC
}

// ─────────────────────────────────────────────────────────────────────────────
// 运行时交互 / 性能预算常量（收敛于此，禁止在组件或渲染器里散落硬编码）
// ─────────────────────────────────────────────────────────────────────────────

/** 桌宠交互常量：轻点/拖拽判别 · 松手惯性 · 部位热区 · 伪随机 seed */
export const PET_INTERACTION = {
  /** 轻点判别：位移 ≤ tapMaxMove(px) 且时长 ≤ tapMaxMs(ms) → 视为「轻点」而非拖动 */
  tapMaxMove: 6,
  tapMaxMs: 380,
  /** 松手惯性滑行（friction=每秒指数衰减率 / minSpeed=停止阈值 px/s）
   *  restitution=触边速度保留系数（贴边 + 轻微回弹）/ maxDt=单步封顶防后台巨步 */
  inertia: { friction: 4.5, minSpeed: 40, restitution: 0.3, maxDt: 1 / 30 },
  /** 部位热区：宠物体框内纵向归一化分界（0=顶，1=底）——头 / 背 / 尾三段 */
  region: { headBelow: 0.42, tailAbove: 0.72 },
  /** 确定性伪随机 seed：同 seed → 同眨眼/抽耳/粒子序列（渲染层禁止 Math.random） */
  seed: 0x9e3779b9,
} as const

/** 帧率感知降级预算：连续低帧 → 降档（关阴影/降采样/减粒子）；连续恢复 → 升档 */
export interface PetPerfConfig {
  /** 采样窗口（秒） */
  sampleSeconds: number
  /** 窗口均帧 < lowFps 视为卡顿 */
  lowFps: number
  /** 窗口均帧 > recoverFps 视为流畅 */
  recoverFps: number
  /** 连续 N 个窗口达标才切换档位（去抖，避免噪声抖动） */
  streak: number
}

export const PET_PERF_BUDGET: PetPerfConfig = {
  sampleSeconds: 2,
  lowFps: 42,
  recoverFps: 56,
  streak: 2,
}

/**
 * 渲染档位 → 具体开关（三档质量的数据驱动单一事实源）。
 *
 * 此前只有 high/low 两档，且开关以散落的 `quality === 'high'` 三元表达式写在渲染器里：
 * 一旦探测判定 low，阴影 + 抗锯齿 + 采样率**一起**全关。中端机（WebGL2 可用但核心数/
 * 内存偏低）只能在「全开」与「全关」之间二选一——这就是「无中间档」的实际后果。
 * medium 档保留阴影（最大视觉收益）而只降抗锯齿、采样率与阴影分辨率。
 *
 * SVG 渲染器（`petSvg.ts`）只消费其中两列：`shadows`（→ 柔光模糊半径的软硬）与
 * `particleBudget`（→ `burst()` 粒子数）；`antialias`/`maxPixelRatio`/`shadowMapSize`
 * 是 WebGL 路径遗留的开关，与 SVG 的形象完整度无关。
 */
export interface PetQualityConfig {
  /** 抗锯齿（MSAA） */
  antialias: boolean
  /** 设备像素比上限（1 = 不做超采样） */
  maxPixelRatio: number
  /** 是否启用阴影贴图 */
  shadows: boolean
  /** 阴影贴图边长（不启用阴影时无意义） */
  shadowMapSize: number
  /** 粒子产出系数（1 = 全量；<1 只减少新粒子，不改池容量） */
  particleBudget: number
  /** 是否启用帧率自适应降档（low 已是省电档，不再自适应） */
  adaptiveDegrade: boolean
}

/** 档位由高到低的顺序（自适应降档沿它逐级下探） */
export const PET_QUALITY_ORDER: readonly PetRenderTier[] = ['high', 'medium', 'low']

export const PET_QUALITY: Record<PetRenderTier, PetQualityConfig> = {
  high: {
    antialias: true,
    maxPixelRatio: 2,
    shadows: true,
    shadowMapSize: 1024,
    particleBudget: 1,
    adaptiveDegrade: true,
  },
  medium: {
    antialias: false,
    maxPixelRatio: 1.5,
    shadows: true,
    // 阴影 pass 与贴图尺寸成正比：中端机靠减半分辨率省掉大头，仍保留投影层次
    shadowMapSize: 512,
    particleBudget: 0.75,
    adaptiveDegrade: true,
  },
  low: {
    antialias: false,
    maxPixelRatio: 1,
    shadows: false,
    shadowMapSize: 512,
    particleBudget: 0.5,
    adaptiveDegrade: false,
  },
}

/** 纯函数：降一档（low 已是最低 → 原样返回） */
export function stepDownQuality(tier: PetRenderTier): PetRenderTier {
  const i = PET_QUALITY_ORDER.indexOf(tier)
  return PET_QUALITY_ORDER[Math.min(i + 1, PET_QUALITY_ORDER.length - 1)]
}

/** 纯函数：升一档（high 已是最高 → 原样返回） */
export function stepUpQuality(tier: PetRenderTier): PetRenderTier {
  const i = PET_QUALITY_ORDER.indexOf(tier)
  return PET_QUALITY_ORDER[Math.max(i - 1, 0)]
}

/**
 * 柔光层参数（当前消费者：`petSvg.buildHalo()` / `petSvg.buildOrnament()` /
 * `haloGlowOpacity()`）。
 *
 * 设计目标不变：光环若只有一圈硬边线，亮起时看不出「四周那种柔光感」。因此环心叠一层
 * **径向渐变 + 高斯模糊**的柔光，尺寸略大于硬环（溢出环外才形成过渡），不透明度恒低于硬环。
 * 数值是 SVG viewBox 坐标下的实测比例，与硬环尺寸联动（改环不必再手调光晕）。
 */
export const PET_GLOW = {
  /** 光晕横向半径 = 硬环横向半径 × 该倍数（>1 才溢出环外，形成柔光过渡） */
  haloRadiusRatio: 1.54,
  /** 光晕纵向半径 = 硬环纵向半径 × 该倍数（椭圆的纵向溢出比横向更明显） */
  haloHeightRatio: 1.71,
  /** 光晕不透明度 = 光环不透明度 × 该倍数（<1：光晕比硬边环更淡） */
  haloOpacityRatio: 0.55,
  /** 头饰中心光晕不透明度（常亮，不随心情变化） */
  ornamentOpacity: 0.45,
} as const

/**
 * 纯函数：光环不透明度 → 柔光层不透明度。
 *
 * 光晕必须与硬边环**同步**亮灭（环不亮时光晕不能残留，否则会出现「无环有光」的鬼影），
 * 且始终比环淡；输入被夹到 [0,1]，异常值不会放大成过曝白团。
 */
export function haloGlowOpacity(haloOpacity: number): number {
  const clamped = Number.isFinite(haloOpacity) ? Math.min(1, Math.max(0, haloOpacity)) : 0
  return clamped * PET_GLOW.haloOpacityRatio
}

/**
 * 粒子参数（数据驱动）。当前唯一消费者是 `petSvg.burst()`：只读 `burstCount`
 * （实际数量再乘档位 `PET_QUALITY[tier].particleBudget`），粒子形状与运动由
 * Web Animations API 现算 —— 所以这里**只保留真正被消费的那一个字段**，
 * 不再留着旧 WebGL 池的观感基线（无执行路径的常量会被当成"还在用"）。
 */
export const PET_PARTICLES = {
  /** 单次 burst 的默认数量 */
  burstCount: 12,
} as const

/** 自适应档位状态（熔断计数 + 当前档位） */
export interface PetTierState {
  /** 当前生效档位（起始值 = 能力探测判定的基础档） */
  tier: PetRenderTier
  /** 连续低帧窗口数 */
  lowStreak: number
  /** 连续高帧窗口数 */
  highStreak: number
}

export function createPetTierState(tier: PetRenderTier): PetTierState {
  return { tier, lowStreak: 0, highStreak: 0 }
}

/**
 * 纯函数：喂入一个采样窗口的平均帧率，返回新的档位状态。
 * - 连续 `streak` 个窗口低于 `lowFps` → **降一档**（high→medium→low）。
 * - 连续 `streak` 个窗口高于 `recoverFps` → **升一档**（low→medium→high）。
 * - 中间地带（噪声区）→ 保持当前档位并清零计数（避免抖动切换）。
 *
 * 与旧布尔版（degraded: true/false）的区别：三档下「降级」不再是两态翻转——
 * high 档掉帧先降到 medium（保住阴影）而不是直接掉到无阴影的 low。
 */
export function stepPetTier(
  state: PetTierState,
  avgFps: number,
  cfg: PetPerfConfig = PET_PERF_BUDGET,
): PetTierState {
  if (avgFps < cfg.lowFps) {
    const lowStreak = state.lowStreak + 1
    return {
      tier: lowStreak >= cfg.streak ? stepDownQuality(state.tier) : state.tier,
      lowStreak,
      highStreak: 0,
    }
  }
  if (avgFps > cfg.recoverFps) {
    const highStreak = state.highStreak + 1
    return {
      tier: highStreak >= cfg.streak ? stepUpQuality(state.tier) : state.tier,
      lowStreak: 0,
      highStreak,
    }
  }
  return { tier: state.tier, lowStreak: 0, highStreak: 0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// 纯校验（坏 Spec 测试期即失败 —— 诚实渲染）
// ─────────────────────────────────────────────────────────────────────────────

/** 校验 Spec；返回错误列表（空 = 合法）。收集全部错误，便于排查。 */
export function validatePetSpec(spec: PetModelSpec): string[] {
  const errors: string[] = []
  if (!spec.id.trim()) errors.push('id 不能为空')
  if (!spec.label.trim()) errors.push('label 不能为空')

  const p = spec.proportions
  const isHumanoid = spec.kind === 'humanoid'
  // 人型：耳/尾/鼻/bodyRadius 不是必须（可以为 0 = 隐形/无）；只校验以下强制正数键：
  const HUMANOID_REQUIRED_POSITIVE = new Set([
    'headRadius', 'eyeScale', 'heightScale',
    'torsoLength', 'legLength', 'armLength', 'neckLen',
  ])
  // 兽型：除了 bodyRadius 外，耳/尾都必须正数；允许 torsoLength 等可空（遗留兼容：0 = 不渲染）
  const BEAST_OPTIONAL_ZERO = new Set([
    'torsoLength', 'legLength', 'armLength', 'neckLen', // 兽型没有人形骨架，这些 legacy 字段可为 0
  ])
  for (const [k, v] of Object.entries(p)) {
    if (typeof v !== 'number') continue
    if (isHumanoid) {
      if (HUMANOID_REQUIRED_POSITIVE.has(k) && !(v > 0)) {
        errors.push(`proportions.${k} 必须为正数（当前 ${v}，人型必需）`)
      }
    } else {
      if (BEAST_OPTIONAL_ZERO.has(k)) continue
      if (!(v > 0)) errors.push(`proportions.${k} 必须为正数（当前 ${v}）`)
    }
  }

  // 心情覆盖完整性：任何 PetMood3D 缺失 → 渲染时静默缺表情（不诚实）
  const mood3d: PetMood3D[] = ['normal', 'happy', 'sleepy', 'working', 'excited', 'sad']
  for (const m of mood3d) {
    const ep = spec.emotionPoses[m]
    if (!ep) {
      errors.push(`emotionPoses 缺少心情 '${m}'`)
      continue
    }
    if (!ep.mouth || !['smile', 'open', 'frown', 'swoon', 'none'].includes(ep.mouth.shape)) {
      errors.push(`emotionPoses['${m}'].mouth.shape 非法`)
    }
    if (typeof ep.blush?.opacity !== 'number' || ep.blush.opacity < 0 || ep.blush.opacity > 1) {
      errors.push(`emotionPoses['${m}'].blush.opacity 须在 0..1`)
    }
    if (typeof ep.pupilScale !== 'number' || ep.pupilScale <= 0) {
      errors.push(`emotionPoses['${m}'].pupilScale 须为正数`)
    }
  }

  const mo = spec.motion
  for (const [k, v] of Object.entries(mo)) {
    if (typeof v === 'object' && v !== null) {
      for (const [kk, vv] of Object.entries(v)) {
        if (typeof vv === 'number' && vv < 0) errors.push(`motion.${k}.${kk} 不能为负`)
      }
    }
  }

  // 外观差异化：未知枚举直接拒绝（渲染器没有分支渲染它——诚实）
  const app = spec.appearance
  if (app) {
    if (app.earStyle && !['cat', 'elf'].includes(app.earStyle)) errors.push('appearance.earStyle 非法')
    if (
      app.ornament &&
      !['daisy', 'blossom', 'lavender', 'clover', 'none'].includes(app.ornament)
    ) errors.push('appearance.ornament 非法')
    if (app.tailStyle && !['puff', 'wisp', 'none'].includes(app.tailStyle)) errors.push('appearance.tailStyle 非法')
    // 发型/裙型：类型上预留了多变体，但渲染器目前**只实现了默认变体**。
    // 明确拒绝而非静默忽略——否则调用方传 twin_tails/frill 会以为生效，实际拿到默认造型。
    if (app.hairStyle && app.hairStyle !== 'long_wave') {
      errors.push(
        `appearance.hairStyle="${app.hairStyle}" 尚未实现（渲染器仅有 long_wave 分支），拒绝以免静默降级`,
      )
    }
    if (app.dressStyle && app.dressStyle !== 'petal_dress') {
      errors.push(
        `appearance.dressStyle="${app.dressStyle}" 尚未实现（渲染器仅有 petal_dress 分支），拒绝以免静默降级`,
      )
    }
    // 翼型三态均已实现（butterfly 四片 / fairy 双蝉翼 / none 隐藏）
    if (app.wingStyle && !['butterfly', 'fairy', 'none'].includes(app.wingStyle)) errors.push('appearance.wingStyle 非法')
    if (app.hasBouquet !== undefined && typeof app.hasBouquet !== 'boolean') {
      errors.push('appearance.hasBouquet 须为布尔')
    }
    for (const k of ['furColor', 'furAccent', 'skin', 'dressColor', 'dressAccent', 'shoeColor'] as const) {
      const v = app[k]
      if (v !== undefined && (typeof v !== 'number' || !(v > 0))) errors.push(`appearance.${k} 须为正颜色数`)
    }
  }
  // kind 必须合法
  if (!['beast', 'humanoid'].includes(spec.kind)) errors.push('spec.kind 必须为 beast 或 humanoid')
  return errors
}

/** 便捷：Spec 是否合法（通常用于测试断言，勿在生产吞掉错误） */
export function isPetSpecValid(spec: PetModelSpec): boolean {
  return validatePetSpec(spec).length === 0
}