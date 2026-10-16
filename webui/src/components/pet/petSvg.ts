/**
 * 桌宠 SVG 渲染器（v4）— 2D 矢量 + 伪 3D 光照，替代原 Three.js/WebGL 路径。
 *
 * ## 为什么是「2D SVG 读出 3D」
 *
 * 立体感不来自真三维，而来自**一致的光照约定 + 层次遮挡 + 动态视差**，这三件事 SVG 全能做，
 * 而且免 WebGL、免包体、免 GPU 上下文丢失，还能被主题令牌完全控制：
 *
 * 1. **光照约定（全局同向）**：主光左上、暗面右下、`accent` 作背光轮廓光。每个材质都由
 *    [`rampOf`] 派生 light/base/shade/deep/rim/line 六层，所有部件共用同一份，绝不各画各的。
 * 2. **体积塑造**：径向渐变（受光点偏移到左上） + 环境遮蔽（关节/贴地处叠软阴影） +
 *    接触投影（地面椭圆随跳跃高度缩放/淡出）。这三样是「贴纸感 → 有体积」的分水岭。
 * 3. **伪 3D 视差**：头部 yaw 不只是转角度 —— 五官整体横移、远侧耳/发片反向偏移、头部
 *    横向压缩；pitch 则压缩纵向并下移五官。侧脸不再是「平面平移」。
 * 4. **材质分层**：头发/裙摆/翅膀各自是「暗部大片 → 主色片 → 高光带 → 轮廓光」四层叠加，
 *    而不是单块纯色。高光带（天使环）是动漫质感的来源。
 *
 * ## 位姿数学复用
 *
 * 逐帧位姿**完全复用** [`computePose`]（既有纯函数引擎）：呼吸/漂浮/眨眼/耳抽动/尾摆/翼振/
 * 光环/蹦跳/揉眼/部位点击衰减/悬停聚焦的数学一行不改，本模块只做「值 → SVG 变换」映射。
 * 因此渲染器可替换，动画语义与既有 780 行 spec 测试保持同一套事实源。
 *
 * ## 诚实边界
 *
 * - 无 WebGL 也能渲染（纯 DOM）；渲染档位只调**特效丰富度**（模糊/粒子预算），不降级形象。
 * - `burst()` 的粒子用 Web Animations API 播完即移除，不常驻 DOM，不无界增长。
 */

import {
  haloGlowOpacity,
  mergeAppearance,
  getPetModelSpec,
  PET_GLOW,
  PET_PARTICLES,
  PET_QUALITY,
  type PetModelSpec,
  type AppearanceSpec,
  type EyeAnchors,
  BEAST_EYE_ANCHORS,
  HUMANOID_EYE_ANCHORS,
} from './petSpec'
import type { PetMood3D, PetRenderTier } from './petContract'
import {
  computePose,
  createSeededRandom,
  type Pose,
  type PoseInput,
  type PetHitRegion,
} from './petPose'
import { NEUTRAL, hexOf, mixRgb, rampOf, shadeRgb, type MaterialRamp } from './petSvgColor'

/** 画布逻辑尺寸（viewBox 边长）；所有几何按此坐标系手写，缩放交给 CSS */
export const PET_SVG_SIZE = 200
/** 角色中轴 x */
const CX = 100
/** 落地线 y（脚底/阴影所在） */
const GROUND_Y = 182
const SVG_NS = 'http://www.w3.org/2000/svg'

/** 渲染器构造参数（与旧 Pet3D 同名同义，宿主可一行替换） */
export interface PetSvgOptions {
  container: HTMLElement
  /** 主题色覆盖（缺省用 spec.accent） */
  accent?: number
  /** 模型规格（缺省 = 云绒猫） */
  spec?: PetModelSpec
  /** 渲染档位：只影响特效预算（模糊/粒子），不影响形象完整度 */
  quality?: PetRenderTier
}

/** 渲染层持有的可动部件引用（逐帧只改这些节点的属性） */
interface PetParts {
  root: SVGGElement
  body: SVGGElement
  head: SVGGElement
  /** 五官整体（yaw 视差横移用；不含耳与后发） */
  face: SVGGElement
  earL: SVGGElement
  earR: SVGGElement
  eyeL: SVGGElement
  eyeR: SVGGElement
  /** 眼睑（眨眼用：高度 = 1 - blinkScaleY） */
  lidL: SVGRectElement
  lidR: SVGRectElement
  pupilL: SVGGElement
  pupilR: SVGGElement
  browL: SVGGElement
  browR: SVGGElement
  mouth: Record<'smile' | 'open' | 'frown' | 'swoon', SVGGElement>
  blush: SVGGElement
  wingL: SVGGElement
  wingR: SVGGElement
  tail: SVGGElement | null
  footL: SVGGElement
  footR: SVGGElement
  ornament: SVGGElement
  halo: SVGGElement
  haloGlow: SVGEllipseElement
  shadow: SVGEllipseElement
  hairBack: SVGGElement | null
  armL: SVGGElement | null
  armR: SVGGElement | null
  dress: SVGGElement | null
  /** 描边宽度基准（线宽随缩放档位；低档略粗，避免小尺寸下断线） */
  fx: SVGGElement
}

const DEG = 180 / Math.PI
const clamp01 = (v: number) => Math.min(1, Math.max(0, v))

/** 创建 SVG 元素（`document` 缺失时抛错由调用方降级；此处不做静默兜底） */
function el<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG_NS, tag)
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v))
  return node
}

/** 椭圆快捷构造 */
function ellipse(
  cx: number,
  cy: number,
  rx: number,
  ry: number,
  attrs: Record<string, string | number> = {},
): SVGEllipseElement {
  return el('ellipse', { cx, cy, rx, ry, ...attrs })
}

/** 路径快捷构造 */
function pathOf(d: string, attrs: Record<string, string | number> = {}): SVGPathElement {
  return el('path', { d, ...attrs })
}

/** 分组快捷构造 */
function group(attrs: Record<string, string | number> = {}): SVGGElement {
  return el('g', attrs)
}

/**
 * 径向渐变（受光点偏移到左上 = 体积感来源）。
 * `fx/fy` 用百分比，默认 34%/28% —— 与线稿的左上主光一致。
 */
function radialGrad(
  defs: SVGDefsElement,
  id: string,
  stops: Array<[offset: number, color: string, opacity?: number]>,
  focal: [number, number] = [34, 28],
): SVGRadialGradientElement {
  const grad = el('radialGradient', { id, cx: '50%', cy: '46%', r: '62%', fx: `${focal[0]}%`, fy: `${focal[1]}%` })
  for (const [offset, color, opacity] of stops) {
    grad.appendChild(el('stop', {
      offset: `${Math.round(offset * 100)}%`,
      'stop-color': color,
      ...(opacity === undefined ? {} : { 'stop-opacity': opacity }),
    }))
  }
  defs.appendChild(grad)
  return grad
}

/** 线性渐变（发梢/裙摆的纵向过渡） */
function linearGrad(
  defs: SVGDefsElement,
  id: string,
  stops: Array<[offset: number, color: string, opacity?: number]>,
  vertical = true,
): SVGLinearGradientElement {
  const grad = el('linearGradient', {
    id,
    x1: '0%', y1: '0%', x2: vertical ? '0%' : '100%', y2: vertical ? '100%' : '0%',
  })
  for (const [offset, color, opacity] of stops) {
    grad.appendChild(el('stop', {
      offset: `${Math.round(offset * 100)}%`,
      'stop-color': color,
      ...(opacity === undefined ? {} : { 'stop-opacity': opacity }),
    }))
  }
  defs.appendChild(grad)
  return grad
}

/** 高斯模糊（环境遮蔽 / 柔光；低档位仍保留但半径略小，避免糊成一团） */
function blurFilter(defs: SVGDefsElement, id: string, sigma: number): void {
  const f = el('filter', { id, x: '-30%', y: '-30%', width: '160%', height: '160%' })
  f.appendChild(el('feGaussianBlur', { stdDeviation: sigma }))
  defs.appendChild(f)
}

/** 材质调色板集合（一次算好，构建期复用） */
interface Palette {
  accent: number
  fur: MaterialRamp
  furAlt: MaterialRamp
  skin: MaterialRamp
  dress: MaterialRamp
  dressAlt: MaterialRamp
  shoe: MaterialRamp
  blush: string
  iris: MaterialRamp
}

function paletteOf(spec: PetModelSpec, accent: number): Palette {
  const a = mergeAppearance(spec)
  return {
    accent,
    fur: rampOf(a.furColor, accent),
    furAlt: rampOf(a.furAccent, accent),
    skin: rampOf(a.skin, accent),
    dress: rampOf(a.dressColor, accent),
    dressAlt: rampOf(a.dressAccent, accent),
    shoe: rampOf(a.shoeColor, accent),
    // 内耳/腮红/唇/鼻：**暖粉**，与冷调毛色形成冷暖对比。
    // 不能从 furAccent 派生（那是冷蓝灰，会让内耳变成灰蓝而不是粉 —— 截图实证）。
    // 数值取自形象参考图的 mistyrose 系（catArt.ts 的调色板注释同源）。
    blush: hexOf(mixRgb(0xf6dfe3, a.skin, 0.25)),
    iris: rampOf(shadeRgb(accent, -0.18), accent),
  }
}

/**
 * 描边宽度（随部件尺寸给，避免小元素线宽喧宾夺主）。
 * 勾线用压暗主色而非黑色 —— 浅色材质配黑线会瞬间发脏。
 */
const LW = { major: 1.5, minor: 1.0, hair: 1.1 } as const

export class PetSvg {
  private readonly container: HTMLElement
  private readonly spec: PetModelSpec
  private readonly palette: Palette
  private readonly tier: PetRenderTier
  private readonly svg: SVGSVGElement
  private readonly defs: SVGDefsElement
  private readonly parts: PetParts
  private readonly uid: string

  // ── 动画状态（与 Pet3D 同语义：外部只调方法，内部自己推时间）──
  private mood: PetMood3D = 'normal'
  private behaviour: 'idle' | 'walk' | 'dance' | 'spin' = 'idle'
  private lastBehaviour = ''
  private facing: 1 | -1 = 1
  private walking = false
  private dancingUntil = 0
  private spinningUntil = 0
  private spinStart = 0
  private hoverWeight = 0
  private hoverTarget = 0
  private pokeRegion: PetHitRegion | null = null
  private pokeAt = 0
  private jumpStart = 0
  private jumpV0 = 0
  private jumpY = 0
  private squash = 1
  private blinkScaleY = 1
  private blinkNextAt = 0
  private blinkUntil = 0
  private doubleBlinkPending = false
  private twitchUntil = 0
  private twitchNextAt = 0
  private walkPhase = 0
  private raf = 0
  private lastT = 0
  private t0 = 0
  private disposed = false
  private readonly rand: () => number
  /** 指针相对容器的归一化位置（-1..1），用于视线跟随 */
  private pointer = { x: 0, y: 0, inside: false }

  constructor(opts: PetSvgOptions) {
    this.container = opts.container
    this.spec = opts.spec ?? getPetModelSpec('cat')
    this.tier = opts.quality ?? 'high'
    this.palette = paletteOf(this.spec, opts.accent ?? this.spec.accent)
    this.rand = createSeededRandom(0x5eed ^ (this.spec.id.length * 2654435761))
    this.uid = `petsvg-${Math.abs(hash32(this.spec.id))}`

    this.svg = el('svg', {
      viewBox: `0 0 ${PET_SVG_SIZE} ${PET_SVG_SIZE}`,
      class: 'pet-svg-canvas',
      role: 'presentation',
      'aria-hidden': 'true',
      // 铺满宿主容器并整体等比缩放：`xMidYMax` 让「脚底/地面」始终贴住容器下沿 ——
      // 桌宠是"站在"它的小格子里的，垂直居中会看起来浮空。
      // 必须显式给 width/height（只给 viewBox 时浏览器会退化成 300×150 一类默认尺寸，
      // 实测把 200 单位图撑成 806px 高、下半身溢出被面板盖住）。
      width: '100%',
      height: '100%',
      preserveAspectRatio: 'xMidYMax meet',
    })
    this.svg.style.display = 'block'
    this.defs = el('defs')
    this.svg.appendChild(this.defs)

    const quality = PET_QUALITY[this.tier]
    // 模糊半径随档位：高档柔光更铺得开；低档缩到 60%，仍保留环境遮蔽（立体感不可省）
    const soft = quality.shadows ? 1 : 0.6
    blurFilter(this.defs, `${this.uid}-ao`, 4.2 * soft)
    blurFilter(this.defs, `${this.uid}-glow`, 7.5 * soft)
    blurFilter(this.defs, `${this.uid}-hairblur`, 3.0 * soft)

    this.buildMaterials()
    this.parts = this.spec.kind === 'humanoid' ? this.buildHumanoid() : this.buildBeast()
    this.svg.appendChild(this.parts.root)
    this.container.appendChild(this.svg)

    this.blinkNextAt = 1.4 + this.rand() * 2.2
    this.twitchNextAt = 2.0 + this.rand() * 3.0
    this.t0 = now()
    this.lastT = this.t0
    this.loop()
  }

  // ────────────────────────────────────────────────────────────────────
  // 材质定义（每个材质一组渐变，统一光照约定）
  // ────────────────────────────────────────────────────────────────────

  private buildMaterials(): void {
    const p = this.palette
    const u = this.uid
    // 主毛/布：受光点左上 → 主色 → 右下暗面（体积的主要来源）
    radialGrad(this.defs, `${u}-fur`, [[0, p.fur.light], [0.46, p.fur.base], [0.82, p.fur.shade], [1, p.fur.deep]])
    radialGrad(this.defs, `${u}-furAlt`, [[0, p.furAlt.light], [0.5, p.furAlt.base], [1, p.furAlt.shade]], [38, 26])
    radialGrad(this.defs, `${u}-skin`, [[0, p.skin.light], [0.58, p.skin.base], [1, p.skin.shade]], [40, 24])
    radialGrad(this.defs, `${u}-dress`, [[0, p.dress.light], [0.5, p.dress.base], [0.86, p.dress.shade], [1, p.dress.deep]])
    radialGrad(this.defs, `${u}-dressAlt`, [[0, p.dressAlt.light], [0.6, p.dressAlt.base], [1, p.dressAlt.shade]], [36, 22])
    radialGrad(this.defs, `${u}-shoe`, [[0, p.shoe.light], [0.6, p.shoe.base], [1, p.shoe.shade]])
    // 眼：虹膜外圈深、内圈亮（「通透感」），瞳孔纯黑另绘
    radialGrad(this.defs, `${u}-iris`, [[0, p.iris.light], [0.55, p.iris.base], [1, p.iris.deep]], [42, 30])
    // 眼白：上方压暗（上眼睑投影）→ 下方留白，是「眼球在眼窝里」的关键
    linearGrad(this.defs, `${u}-sclera`, [[0, p.skin.shade], [0.34, NEUTRAL.white], [1, NEUTRAL.white]])
    // 发：顶亮 → 中段主色 → 发梢 accent（既立体又有配色呼应）
    linearGrad(this.defs, `${u}-hair`, [[0, p.fur.light], [0.38, p.fur.base], [0.78, p.fur.shade], [1, p.furAlt.base]])
    // 高光带（天使环）
    linearGrad(this.defs, `${u}-hairShine`, [[0, NEUTRAL.white], [0.5, NEUTRAL.white], [1, NEUTRAL.white]])
    // 裙：上亮下深 + 薄纱层
    linearGrad(this.defs, `${u}-skirt`, [[0, p.dress.light], [0.45, p.dress.base], [1, p.dress.shade]])
    linearGrad(this.defs, `${u}-tulle`, [[0, p.dressAlt.light], [1, p.dressAlt.shade]])
    // 翅：外缘 accent 亮、内侧收淡
    radialGrad(this.defs, `${u}-wing`, [[0, p.furAlt.light], [0.5, p.dressAlt.base], [1, p.dressAlt.deep]], [28, 22])
    // 轮廓光（背光边缘）
    linearGrad(this.defs, `${u}-rim`, [[0, p.fur.rim], [1, p.dressAlt.rim]], false)
    // 接触投影 / 环境遮蔽
    radialGrad(this.defs, `${u}-groundShadow`, [[0, NEUTRAL.black], [0.55, NEUTRAL.black], [1, NEUTRAL.black]])
    radialGrad(this.defs, `${u}-ao`, [[0, NEUTRAL.black], [1, NEUTRAL.black]], [50, 50])
    // 光环柔光
    radialGrad(this.defs, `${u}-haloGlow`, [
      [0, hexOf(shadeRgb(p.accent, 0.45))],
      [0.42, hexOf(p.accent)],
      [1, hexOf(p.accent)],
    ], [50, 50])
    radialGrad(this.defs, `${u}-halo`, [[0, hexOf(shadeRgb(p.accent, 0.5))], [0.62, hexOf(p.accent)], [1, hexOf(shadeRgb(p.accent, -0.2))]])
  }

  private get anchors(): EyeAnchors {
    return this.spec.kind === 'humanoid' ? HUMANOID_EYE_ANCHORS : BEAST_EYE_ANCHORS
  }

  // ────────────────────────────────────────────────────────────────────
  // 通用部件（两分支共用；单位 = viewBox 坐标）
  // ────────────────────────────────────────────────────────────────────

  /** 接触投影：任何角色都有的「落地锚」，跳跃时收缩淡出（悬浮感） */
  private buildGroundShadow(): SVGEllipseElement {
    const g = ellipse(CX, GROUND_Y + 2, 42, 10, {
      fill: `url(#${this.uid}-groundShadow)`,
      opacity: 0.28,
      filter: `url(#${this.uid}-ao)`,
    })
    return g
  }

  /** 光环 + 柔光（working/excited/happy 亮起；柔光尺寸/不透明度均按 PET_GLOW 由硬环派生） */
  private buildHalo(): { g: SVGGElement; glow: SVGEllipseElement; ring: SVGEllipseElement } {
    const g = group({ opacity: 0 })
    const rx = 26
    const ry = 7
    const glow = ellipse(CX, 34, rx * PET_GLOW.haloRadiusRatio, ry * PET_GLOW.haloHeightRatio, {
      fill: `url(#${this.uid}-haloGlow)`,
      filter: `url(#${this.uid}-glow)`,
    })
    const ring = ellipse(CX, 34, rx, ry, { fill: 'none', stroke: `url(#${this.uid}-halo)`, 'stroke-width': 2.2 })
    g.appendChild(glow)
    g.appendChild(ring)
    return { g, glow, ring }
  }

  /** 头饰（每皮肤不同花型；花心带柔光） */
  private buildOrnament(appearance: AppearanceSpec, headY: number): SVGGElement {
    const g = group()
    const kinds = { daisy: 8, blossom: 5, lavender: 4, clover: 4, none: 0 } as const
    const petals = kinds[appearance.ornament] ?? 0
    if (petals === 0) return g
    const petal = appearance.ornament === 'lavender' ? 1.5 : 2.6
    const petalR = appearance.ornament === 'clover' ? 2.1 : petal
    const fill = appearance.ornament === 'clover'
      ? hexOf(shadeRgb(appearance.dressAccent, -0.08))
      : hexOf(shadeRgb(appearance.furAccent, 0.2))
    const core = appearance.ornament === 'daisy'
      ? hexOf(shadeRgb(this.spec.accent, 0.25))
      : hexOf(shadeRgb(appearance.dressColor, -0.1))
    for (let i = 0; i < petals; i++) {
      const a = (i / petals) * Math.PI * 2
      g.appendChild(ellipse(
        Math.cos(a) * petalR * 1.5,
        Math.sin(a) * petalR * 1.5,
        petalR,
        petalR * 1.25,
        { fill, opacity: 0.95, transform: `rotate(${(a * DEG).toFixed(1)})` },
      ))
    }
    g.appendChild(ellipse(0, 0, petalR * 1.1, petalR * 1.1, { fill: core }))
    g.appendChild(ellipse(0, 0, petalR * 2.4, petalR * 2.4, {
      fill: `url(#${this.uid}-haloGlow)`,
      opacity: PET_GLOW.ornamentOpacity * 0.5,
      filter: `url(#${this.uid}-glow)`,
    }))
    g.setAttribute('transform', `translate(0 ${headY - 26})`)
    return g
  }

  /** 眼（眼白 + 虹膜 + 瞳孔 + 双高光 + 眼睑）：动漫质感的核心部件 */
  private buildEye(x: number, y: number, rx: number, ry: number, _side: 'L' | 'R') {
    const u = this.uid
    // 局部坐标：本组挂在 head/face 之下，head 已带绝对位移 —— 这里**不得**再加 CX
    // （曾因此把双眼整体推右 100 单位、右眼被裁出 viewBox）。
    const g = group({ transform: `translate(${x} ${y})` })
    const sclera = ellipse(0, 0, rx, ry, { fill: `url(#${u}-sclera)` })
    // 上眼睑投影：眼球上部一小片暗，避免「白饼上贴圆点」
    const lidShade = pathOf(
      `M ${-rx} ${-ry * 0.52} A ${rx} ${ry} 0 0 1 ${rx} ${-ry * 0.52} Q 0 ${-ry * 0.1} ${-rx} ${-ry * 0.52} Z`,
      { fill: this.palette.skin.shade, opacity: 0.5 },
    )
    // 视线组：虹膜/瞳孔/高光一起位移（瞳孔偏移作用在这一层）
    const gaze = group()
    gaze.appendChild(ellipse(0, ry * 0.04, rx * 0.74, ry * 0.78, { fill: `url(#${u}-iris)` }))
    gaze.appendChild(ellipse(0, ry * 0.04, rx * 0.34, ry * 0.42, { fill: NEUTRAL.black, opacity: 0.88 }))
    // 高光：左上大 + 右下小（两点高光 = 湿润感）
    gaze.appendChild(ellipse(-rx * 0.3, -ry * 0.34, rx * 0.24, ry * 0.2, { fill: NEUTRAL.white, opacity: 0.95 }))
    gaze.appendChild(ellipse(rx * 0.32, ry * 0.34, rx * 0.12, ry * 0.1, { fill: NEUTRAL.white, opacity: 0.6 }))
    const outline = ellipse(0, 0, rx, ry, {
      fill: 'none', stroke: this.palette.fur.line, 'stroke-width': LW.minor, opacity: 0.55,
    })
    // 眼睑（眨眼）：与毛/肤色同色，从上方盖下来
    const lid = el('rect', {
      x: -rx * 1.05, y: -ry * 1.3, width: rx * 2.1, height: 0,
      fill: `url(#${u}-${this.spec.kind === 'humanoid' ? 'skin' : 'fur'})`,
    })
    g.appendChild(sclera)
    g.appendChild(lidShade)
    g.appendChild(gaze)
    g.appendChild(outline)
    g.appendChild(lid)
    return { g, lid, gaze }
  }

  // ────────────────────────────────────────────────────────────────────
  // 兽形（云绒猫）：圆润大头 + 尖耳 + 绒球尾
  // ────────────────────────────────────────────────────────────────────

  private buildBeast(): PetParts {
    const u = this.uid
    const p = this.palette
    const root = group()

    const shadow = this.buildGroundShadow()
    root.appendChild(shadow)

    const halo = this.buildHalo()
    root.appendChild(halo.g)

    // 尾（在身体之后）：三节绒球 + 链条波动
    const tail = group({ transform: `translate(${CX + 30} ${GROUND_Y - 30})` })
    for (let i = 0; i < 3; i++) {
      tail.appendChild(ellipse(i * 7, -i * 7, 11 - i * 1.6, 11 - i * 1.6, {
        fill: `url(#${u}-fur)`,
        stroke: p.fur.line, 'stroke-width': LW.minor, opacity: 0.99,
      }))
    }
    root.appendChild(tail)

    // 身体 + 肚皮 + 前爪
    const body = group({ transform: `translate(${CX} ${GROUND_Y - 30})` })
    body.appendChild(ellipse(0, 0, 40, 32, {
      fill: `url(#${u}-fur)`, stroke: p.fur.line, 'stroke-width': LW.major,
    }))
    body.appendChild(ellipse(0, 6, 25, 21, { fill: `url(#${u}-furAlt)`, opacity: 0.5 }))
    // 环境遮蔽：身体下缘的贴地暗部
    body.appendChild(ellipse(0, 26, 30, 8, { fill: `url(#${u}-ao)`, opacity: 0.22, filter: `url(#${u}-ao)` }))
    const footL = group({ transform: 'translate(-20 30)' })
    footL.appendChild(ellipse(0, 0, 13, 9, { fill: `url(#${u}-fur)`, stroke: p.fur.line, 'stroke-width': LW.minor }))
    footL.appendChild(ellipse(0, 2, 8, 4.4, { fill: `url(#${u}-furAlt)`, opacity: 0.55 }))
    const footR = group({ transform: 'translate(20 30)' })
    footR.appendChild(ellipse(0, 0, 13, 9, { fill: `url(#${u}-fur)`, stroke: p.fur.line, 'stroke-width': LW.minor }))
    footR.appendChild(ellipse(0, 2, 8, 4.4, { fill: `url(#${u}-furAlt)`, opacity: 0.55 }))
    body.appendChild(footL)
    body.appendChild(footR)
    // 项圈（accent 点缀，带高光；这是主题色在兽形上的落点）
    body.appendChild(ellipse(0, -22, 21, 6, { fill: `url(#${u}-dressAlt)`, opacity: 0.9 }))
    body.appendChild(ellipse(0, -18, 5, 5, { fill: hexOf(shadeRgb(p.accent, 0.3)), stroke: p.fur.line, 'stroke-width': LW.minor }))
    root.appendChild(body)

    // 头（先画头圆，再叠耳/五官）
    const head = group({ transform: `translate(${CX} ${GROUND_Y - 92})` })
    head.appendChild(ellipse(0, 0, 43, 40, {
      fill: `url(#${u}-fur)`, stroke: p.fur.line, 'stroke-width': LW.major,
    }))
    // 脸部提亮（口鼻区），让大头有「正面朝向光」的层次
    head.appendChild(ellipse(0, 5, 27, 24, { fill: `url(#${u}-furAlt)`, opacity: 0.42 }))
    // 轮廓光（背光边缘，右侧一圈 accent 冷光）
    head.appendChild(pathOf(
      'M 24 -30 A 43 40 0 0 1 40 12 A 46 44 0 0 0 24 -30 Z',
      { fill: `url(#${u}-rim)`, opacity: 0.55, filter: `url(#${u}-hairblur)` },
    ))
    // 环境遮蔽：刘海/耳根投在额头上的软阴影（让"平涂的脸"出现明暗分界）
    head.appendChild(ellipse(0, -26, 34, 13, {
      fill: `url(#${u}-ao)`, opacity: 0.2, filter: `url(#${u}-hairblur)`,
    }))
    // **耳必须叠在头圆之上**：先前画在头圆之前且锚点在头轮廓内部，整只耳被头盖住，
    // 剪影上等于没有耳朵（截图实证：只见一个头的圆盘）。
    const earL = this.buildCatEar(-1)
    const earR = this.buildCatEar(1)
    head.appendChild(earL)
    head.appendChild(earR)

    const face = group()
    const anchors = this.anchors
    const eyeSize = { rx: 10.5 * Math.min(1.5, this.spec.proportions.eyeScale), ry: 12.5 * Math.min(1.5, this.spec.proportions.eyeScale) }
    // 眼窝环境遮蔽：让大眼睛「嵌进」毛脸里，而不是浮在表面
    for (const s of [-1, 1] as const) {
      face.appendChild(ellipse(s * anchors.pupilR.x * 100, 6, 13, 12, {
        fill: `url(#${u}-ao)`, opacity: 0.15, filter: `url(#${u}-hairblur)`,
      }))
    }
    const eL = this.buildEye(anchors.pupilL.x * 100, 6, eyeSize.rx, eyeSize.ry, 'L')
    const eR = this.buildEye(anchors.pupilR.x * 100, 6, eyeSize.rx, eyeSize.ry, 'R')
    face.appendChild(eL.g)
    face.appendChild(eR.g)
    // 眉（兽形淡而短）
    const browL = group({ transform: 'translate(-19 -14)' })
    browL.appendChild(pathOf('M -7 0 Q 0 -3.4 7 -0.6', { fill: 'none', stroke: p.fur.line, 'stroke-width': 1.8, 'stroke-linecap': 'round', opacity: 0.7 }))
    const browR = group({ transform: 'translate(19 -14)' })
    browR.appendChild(pathOf('M -7 -0.6 Q 0 -3.4 7 0', { fill: 'none', stroke: p.fur.line, 'stroke-width': 1.8, 'stroke-linecap': 'round', opacity: 0.7 }))
    face.appendChild(browL)
    face.appendChild(browR)
    // 鼻（圆角三角，柔和粉） + 人中
    face.appendChild(pathOf('M -5 19 Q 0 15.6 5 19 Q 0 25 -5 19 Z', { fill: p.blush }))
    face.appendChild(pathOf('M 0 24 L 0 27', { fill: 'none', stroke: p.fur.line, 'stroke-width': 1.2, opacity: 0.6 }))
    // 嘴（四形态）
    const mouth = {} as Record<'smile' | 'open' | 'frown' | 'swoon', SVGGElement>
    mouth.smile = group()
    mouth.smile.appendChild(pathOf('M -9 28 Q -4.5 32.5 0 28 Q 4.5 32.5 9 28', {
      fill: 'none', stroke: p.fur.line, 'stroke-width': 1.6, 'stroke-linecap': 'round',
    }))
    mouth.open = group({ opacity: 0 })
    mouth.open.appendChild(ellipse(0, 30, 6.4, 5.2, { fill: p.blush }))
    mouth.open.appendChild(ellipse(0, 32, 4.2, 2.4, { fill: hexOf(shadeRgb(this.spec.accent, -0.42)), opacity: 0.75 }))
    mouth.frown = group({ opacity: 0 })
    mouth.frown.appendChild(pathOf('M -8 31 Q 0 25.5 8 31', {
      fill: 'none', stroke: p.fur.line, 'stroke-width': 1.6, 'stroke-linecap': 'round',
    }))
    mouth.swoon = group({ opacity: 0 })
    mouth.swoon.appendChild(pathOf('M -8 28 Q 0 33 8 28 Q 0 30 -8 28 Z', { fill: p.blush }))
    for (const m of Object.values(mouth)) face.appendChild(m)
    // 腮红
    const blush = group({ opacity: 0.5 })
    blush.appendChild(ellipse(-24, 14, 7.5, 4.6, { fill: p.blush, opacity: 0.85 }))
    blush.appendChild(ellipse(24, 14, 7.5, 4.6, { fill: p.blush, opacity: 0.85 }))
    face.appendChild(blush)
    // 胡须
    for (const s of [-1, 1]) {
      for (const k of [-4, 0, 4]) {
        face.appendChild(pathOf(`M ${s * 20} ${20 + k * 0.5} L ${s * 40} ${17 + k}`, {
          fill: 'none', stroke: p.fur.line, 'stroke-width': 1, opacity: 0.42, 'stroke-linecap': 'round',
        }))
      }
    }
    head.appendChild(face)

    const ornament = this.buildOrnament(mergeAppearance(this.spec), 0)
    head.appendChild(ornament)
    root.appendChild(head)

    // 兽形无臂/无裙/无后发（守卫式 null：applyPose 跳过）
    const fx = this.parts_fx()
    root.appendChild(fx)
    return {
      root, body, head, face, earL, earR,
      eyeL: eL.g, eyeR: eR.g, lidL: eL.lid, lidR: eR.lid,
      pupilL: eL.gaze, pupilR: eR.gaze,
      browL, browR, mouth, blush,
      wingL: group(), wingR: group(),
      tail, footL, footR, ornament,
      halo: halo.g, haloGlow: halo.glow, shadow,
      hairBack: null, armL: null, armR: null, dress: null,
      fx,
    }
  }

  /** 猫耳：外耳（毛色 · 带内阴影）+ 内耳（粉） */
  private buildCatEar(side: -1 | 1): SVGGElement {
    const u = this.uid
    const p = this.palette
    const s = side
    // 锚点在头轮廓**外侧上方**，耳形向外张开 —— 这样整只耳都在剪影上可见（不再被头盖住）
    const g = group({ transform: `translate(${s * 30} -32)` })
    g.appendChild(pathOf(
      `M 0 0 Q ${s * 3} -24 ${s * 19} -30 Q ${s * 24} -8 ${s * 16} 6 Z`,
      { fill: `url(#${u}-fur)`, stroke: p.fur.line, 'stroke-width': LW.major },
    ))
    g.appendChild(pathOf(
      `M ${s * 6} 2 Q ${s * 9} -16 ${s * 16} -20 Q ${s * 17} -3 ${s * 12} 4 Z`,
      { fill: p.blush, opacity: 0.92 },
    ))
    g.setAttribute('data-side', side < 0 ? 'L' : 'R')
    return g
  }

  // ────────────────────────────────────────────────────────────────────
  // 人形（精灵花嫁）：长波浪发 + 花瓣嫁裙 + 蝶翼 + 捧花
  // ────────────────────────────────────────────────────────────────────

  private buildHumanoid(): PetParts {
    const u = this.uid
    const p = this.palette
    const a = mergeAppearance(this.spec)
    const root = group()
    const shadow = this.buildGroundShadow()
    root.appendChild(shadow)

    // 蝶翼（在身体之后；四片 = 上大下小，半透 + accent 发光）
    const wingL = this.buildButterfly(-1, a)
    const wingR = this.buildButterfly(1, a)
    root.appendChild(wingL)
    root.appendChild(wingR)

    const halo = this.buildHalo()
    root.appendChild(halo.g)

    // 后发（长波浪主体，垂到裙摆之下；三层：暗部大形 → 主色 → 高光带）
    const hairBack = group({ transform: `translate(${CX} ${GROUND_Y - 108})` })
    const hairD = 'M -26 -18 C -40 6 -42 40 -34 66 C -28 84 -14 88 -6 84 C -16 62 -14 30 -8 4 Z'
    hairBack.appendChild(pathOf(hairD, { fill: `url(#${u}-hair)`, stroke: p.fur.line, 'stroke-width': LW.hair, opacity: 0.98 }))
    const hairR = 'M 26 -18 C 40 6 42 40 34 66 C 28 84 14 88 6 84 C 16 62 14 30 8 4 Z'
    hairBack.appendChild(pathOf(hairR, { fill: `url(#${u}-hair)`, stroke: p.fur.line, 'stroke-width': LW.hair, opacity: 0.98 }))
    // 发梢发光（accen 渐层，静止时也能读出体积）
    hairBack.appendChild(pathOf('M -32 58 C -26 78 -14 86 -6 82 C -14 74 -20 66 -24 54 Z', {
      fill: `url(#${u}-furAlt)`, opacity: 0.5, filter: `url(#${u}-hairblur)`,
    }))
    hairBack.appendChild(pathOf('M 32 58 C 26 78 14 86 6 82 C 14 74 20 66 24 54 Z', {
      fill: `url(#${u}-furAlt)`, opacity: 0.5, filter: `url(#${u}-hairblur)`,
    }))
    root.appendChild(hairBack)

    // 身体：腿 → 裙（三层花瓣 + 薄纱）→ 上身 → 手臂 → 捧花
    const body = group({ transform: `translate(${CX} ${GROUND_Y - 62})` })
    const footL = group({ transform: 'translate(-8 56)' })
    const footR = group({ transform: 'translate(8 56)' })
    for (const [g, side] of [[footL, -1], [footR, 1]] as const) {
      g.appendChild(pathOf('M -3 0 L 3 0 L 3 -22 L -3 -22 Z', { fill: `url(#${u}-skin)`, opacity: 0.9 }))
      g.appendChild(ellipse(side * 0.6, 1.5, 6.4, 3.6, { fill: `url(#${u}-shoe)`, stroke: p.shoe.line, 'stroke-width': LW.minor }))
    }
    body.appendChild(footL)
    body.appendChild(footR)

    // 裙：薄纱拖尾（最后） + 三层花瓣（下→上收窄），每层带内阴影
    const dress = group()
    dress.appendChild(pathOf('M -34 40 C -46 46 -40 58 -28 58 L 28 58 C 40 58 46 46 34 40 Z', {
      fill: `url(#${u}-tulle)`, opacity: 0.42, filter: `url(#${u}-hairblur)`,
    }))
    const petals = [
      { y: 44, w: 38, h: 14, fill: `url(#${u}-skirt)`, op: 0.97 },
      { y: 32, w: 32, h: 14, fill: `url(#${u}-dress)`, op: 0.98 },
      { y: 20, w: 25, h: 13, fill: `url(#${u}-skirt)`, op: 0.98 },
    ]
    for (const [i, s] of petals.entries()) {
      dress.appendChild(pathOf(
        `M ${-s.w} ${s.y} C ${-s.w * 0.6} ${s.y + s.h} ${s.w * 0.6} ${s.y + s.h} ${s.w} ${s.y} C ${s.w * 0.5} ${s.y - 7} ${-s.w * 0.5} ${s.y - 7} ${-s.w} ${s.y} Z`,
        { fill: s.fill, opacity: s.op, stroke: p.dress.line, 'stroke-width': LW.minor },
      ))
      // 花瓣层次：每层下缘的软阴影（层与层之间的分离感）
      dress.appendChild(pathOf(
        `M ${-s.w * 0.9} ${s.y + 1} C ${-s.w * 0.5} ${s.y + s.h * 0.5} ${s.w * 0.5} ${s.y + s.h * 0.5} ${s.w * 0.9} ${s.y + 1}`,
        { fill: 'none', stroke: p.dress.deep, 'stroke-width': 1.6, opacity: 0.35, filter: `url(#${u}-hairblur)` },
      ))
      if (i === 0) break
    }
    // 腰封 + 缎带结（accent）
    dress.appendChild(pathOf('M -17 16 C -8 21 8 21 17 16 L 15 8 C 7 12 -7 12 -15 8 Z', {
      fill: `url(#${u}-dressAlt)`, stroke: p.dress.line, 'stroke-width': LW.minor,
    }))
    dress.appendChild(ellipse(0, 14, 3.4, 3.4, { fill: hexOf(shadeRgb(p.accent, 0.28)) }))
    body.appendChild(dress)

    // 上身 + 颈饰
    body.appendChild(pathOf('M -13 10 C -11 -12 11 -12 13 10 C 6 15 -6 15 -13 10 Z', {
      fill: `url(#${u}-dress)`, stroke: p.dress.line, 'stroke-width': LW.major,
    }))
    body.appendChild(pathOf('M -9 -6 C -4 -3 4 -3 9 -6', {
      fill: 'none', stroke: p.dress.deep, 'stroke-width': 1.2, opacity: 0.4,
    }))
    body.appendChild(ellipse(0, -12, 6.4, 2.6, { fill: `url(#${u}-dressAlt)` }))
    body.appendChild(ellipse(0, -12, 1.8, 1.8, { fill: hexOf(shadeRgb(p.accent, 0.34)) }))

    // 手臂（左臂轻抬，右臂捧花）
    const armL = group({ transform: 'translate(-13 -4)' })
    armL.appendChild(pathOf('M 0 0 C -10 6 -14 16 -12 24', {
      fill: 'none', stroke: `url(#${u}-skin)`, 'stroke-width': 5.4, 'stroke-linecap': 'round',
    }))
    armL.appendChild(ellipse(-12, 25, 3.4, 3.4, { fill: `url(#${u}-skin)` }))
    const armR = group({ transform: 'translate(13 -4)' })
    armR.appendChild(pathOf('M 0 0 C 10 6 14 15 12 22', {
      fill: 'none', stroke: `url(#${u}-skin)`, 'stroke-width': 5.4, 'stroke-linecap': 'round',
    }))
    // 捧花（三朵 + 缎带）
    if (a.hasBouquet) {
      const bouquet = group({ transform: 'translate(12 22)' })
      for (const [dx, dy, r] of [[-5, -4, 4.4], [2, -7, 4.8], [6, -1, 4.2]] as const) {
        for (let i = 0; i < 5; i++) {
          const ang = (i / 5) * Math.PI * 2
          bouquet.appendChild(ellipse(dx + Math.cos(ang) * r * 0.9, dy + Math.sin(ang) * r * 0.9, r * 0.62, r * 0.7, {
            fill: `url(#${u}-dressAlt)`, opacity: 0.95,
          }))
        }
        bouquet.appendChild(ellipse(dx, dy, r * 0.5, r * 0.5, { fill: hexOf(shadeRgb(p.accent, 0.3)) }))
      }
      bouquet.appendChild(pathOf('M -4 6 L 4 6 L 2 16 L -2 16 Z', { fill: `url(#${u}-dressAlt)`, opacity: 0.9 }))
      armR.appendChild(bouquet)
    }
    body.appendChild(armL)
    body.appendChild(armR)
    root.appendChild(body)

    // 头：后发已画 → 颈 → 脸 → 五官 → 前发（刘海/侧发）→ 头饰
    const head = group({ transform: `translate(${CX} ${GROUND_Y - 110})` })
    const earL = this.buildElfEar(-1)
    const earR = this.buildElfEar(1)
    head.appendChild(earL)
    head.appendChild(earR)
    head.appendChild(pathOf('M -5 -26 L 5 -26 L 6 -18 L -6 -18 Z', { fill: `url(#${u}-skin)` }))
    head.appendChild(ellipse(0, 0, 30, 31, {
      fill: `url(#${u}-skin)`, stroke: p.skin.line, 'stroke-width': LW.major,
    }))
    // 下颌收窄（Q 版脸型：上宽下窄）+ 腮部提亮
    head.appendChild(pathOf('M -26 6 C -20 24 -10 31 0 31 C 10 31 20 24 26 6 C 14 14 -14 14 -26 6 Z', {
      fill: `url(#${u}-skin)`, opacity: 0.9,
    }))
    // 轮廓光
    head.appendChild(pathOf('M 16 -24 A 30 31 0 0 1 28 8 A 32 33 0 0 0 16 -24 Z', {
      fill: `url(#${u}-rim)`, opacity: 0.5, filter: `url(#${u}-hairblur)`,
    }))
    // 刘海投在额头的软阴影：平涂脸 → 出现明暗分界（人形尤其明显，因为刘海面积大）
    head.appendChild(ellipse(0, -20, 27, 12, {
      fill: `url(#${u}-ao)`, opacity: 0.2, filter: `url(#${u}-hairblur)`,
    }))

    const face = group()
    const anchors = this.anchors
    const eyeScale = Math.min(1.5, this.spec.proportions.eyeScale)
    // 眼窝环境遮蔽：眼球嵌进脸里（否则大眼睛像贴在脸上的两块"贴片"）
    for (const s of [-1, 1] as const) {
      face.appendChild(ellipse(s * anchors.pupilR.x * 100, 4, 12, 11, {
        fill: `url(#${u}-ao)`, opacity: 0.16, filter: `url(#${u}-hairblur)`,
      }))
    }
    const eL = this.buildEye(anchors.pupilL.x * 100, 4, 9.6 * eyeScale, 11.4 * eyeScale, 'L')
    const eR = this.buildEye(anchors.pupilR.x * 100, 4, 9.6 * eyeScale, 11.4 * eyeScale, 'R')
    // 睫毛（上眼睑加粗黑线 + 外眼角上挑，动漫眼的关键）
    for (const [g, s] of [[eL.g, -1], [eR.g, 1]] as const) {
      g.appendChild(pathOf(
        `M ${-9.6 * eyeScale} ${-4.2 * eyeScale} Q 0 ${-12.4 * eyeScale} ${9.6 * eyeScale} ${-4.2 * eyeScale}`,
        { fill: 'none', stroke: NEUTRAL.black, 'stroke-width': 2.6, 'stroke-linecap': 'round', opacity: 0.9 },
      ))
      g.appendChild(pathOf(
        `M ${s * 8.6 * eyeScale} ${-6 * eyeScale} Q ${s * 12.6 * eyeScale} ${-9.4 * eyeScale} ${s * 14.6 * eyeScale} ${-8 * eyeScale}`,
        { fill: 'none', stroke: NEUTRAL.black, 'stroke-width': 2.2, 'stroke-linecap': 'round', opacity: 0.85 },
      ))
    }
    face.appendChild(eL.g)
    face.appendChild(eR.g)
    const browL = group({ transform: 'translate(-16 -17)' })
    browL.appendChild(pathOf('M -8 1.4 Q 0 -3.4 8 -0.4', { fill: 'none', stroke: p.furAlt.deep, 'stroke-width': 1.9, 'stroke-linecap': 'round', opacity: 0.85 }))
    const browR = group({ transform: 'translate(16 -17)' })
    browR.appendChild(pathOf('M -8 -0.4 Q 0 -3.4 8 1.4', { fill: 'none', stroke: p.furAlt.deep, 'stroke-width': 1.9, 'stroke-linecap': 'round', opacity: 0.85 }))
    face.appendChild(browL)
    face.appendChild(browR)
    face.appendChild(pathOf('M -2.4 17 Q 0 19.4 2.4 17', { fill: 'none', stroke: p.skin.deep, 'stroke-width': 1.2, opacity: 0.6 }))
    const mouth = {} as Record<'smile' | 'open' | 'frown' | 'swoon', SVGGElement>
    mouth.smile = group()
    mouth.smile.appendChild(pathOf('M -5 23 Q 0 26.4 5 23', { fill: 'none', stroke: p.blush, 'stroke-width': 1.7, 'stroke-linecap': 'round' }))
    mouth.open = group({ opacity: 0 })
    mouth.open.appendChild(ellipse(0, 24, 4.4, 4.2, { fill: p.blush }))
    mouth.open.appendChild(pathOf('M -3 26 Q 0 28 3 26 Z', { fill: NEUTRAL.white, opacity: 0.7 }))
    mouth.frown = group({ opacity: 0 })
    mouth.frown.appendChild(pathOf('M -5 26 Q 0 21.6 5 26', { fill: 'none', stroke: p.blush, 'stroke-width': 1.7, 'stroke-linecap': 'round' }))
    mouth.swoon = group({ opacity: 0 })
    mouth.swoon.appendChild(pathOf('M -5 23 Q 0 27.4 5 23 Q 0 25 -5 23 Z', { fill: p.blush }))
    for (const m of Object.values(mouth)) face.appendChild(m)
    const blush = group({ opacity: 0.5 })
    blush.appendChild(ellipse(-17, 12, 6.6, 4, { fill: p.blush, opacity: 0.8, filter: `url(#${u}-hairblur)` }))
    blush.appendChild(ellipse(17, 12, 6.6, 4, { fill: p.blush, opacity: 0.8, filter: `url(#${u}-hairblur)` }))
    face.appendChild(blush)
    head.appendChild(face)

    // 前发：刘海（三层）+ 侧发 + 高光带（天使环）
    const bangs = group()
    bangs.appendChild(pathOf('M -30 -8 C -30 -28 -16 -36 0 -36 C 16 -36 30 -28 30 -8 C 24 -20 14 -24 6 -20 C 2 -30 -6 -32 -14 -26 C -22 -22 -26 -16 -30 -8 Z', {
      fill: `url(#${u}-hair)`, stroke: p.fur.line, 'stroke-width': LW.hair,
    }))
    bangs.appendChild(pathOf('M -26 -10 C -20 -22 -8 -26 2 -22 C -8 -18 -18 -14 -26 -10 Z', {
      fill: `url(#${u}-furAlt)`, opacity: 0.42,
    }))
    bangs.appendChild(pathOf('M -20 -22 C -6 -30 12 -30 24 -22 C 12 -24 -6 -24 -20 -22 Z', {
      fill: `url(#${u}-hairShine)`, opacity: 0.34, filter: `url(#${u}-hairblur)`,
    }))
    bangs.appendChild(pathOf('M -30 -8 C -34 4 -34 18 -30 30 C -26 18 -27 4 -24 -8 Z', {
      fill: `url(#${u}-hair)`, stroke: p.fur.line, 'stroke-width': LW.hair,
    }))
    bangs.appendChild(pathOf('M 30 -8 C 34 4 34 18 30 30 C 26 18 27 4 24 -8 Z', {
      fill: `url(#${u}-hair)`, stroke: p.fur.line, 'stroke-width': LW.hair,
    }))
    head.appendChild(bangs)

    const ornament = this.buildOrnament(a, 0)
    head.appendChild(ornament)
    root.appendChild(head)

    const fx = this.parts_fx()
    root.appendChild(fx)
    return {
      root, body, head, face, earL, earR,
      eyeL: eL.g, eyeR: eR.g, lidL: eL.lid, lidR: eR.lid,
      pupilL: eL.gaze, pupilR: eR.gaze,
      browL, browR, mouth, blush,
      wingL, wingR,
      tail: null, footL, footR, ornament,
      halo: halo.g, haloGlow: halo.glow, shadow,
      hairBack, armL, armR, dress,
      fx,
    }
  }

  /** 蝶翼（单侧两片：上大下小；半透 + accent 渐变）
   *
   * 锚点取在**肩上方偏外**（GROUND_Y - 120，即头部高度），并让翼展明显超出头部轮廓 ——
   * 先前锚点偏低、翼展偏小，翅膀整片被头挡住，看起来像两块"贴在耳边的圆斑"而不是翅膀。
   */
  private buildButterfly(side: -1 | 1, _a: AppearanceSpec): SVGGElement {
    const u = this.uid
    const s = side
    const g = group({ transform: `translate(${CX + s * 14} ${GROUND_Y - 120})` })
    // 上翅：向**上外**扫出一个带尖角的翼形（尖端到 ±54 / -34）。
    // 关键是尖角与翼展：先前是短而圆的叶片形 + 一大团圆光，读起来像"贴耳圆斑"而不是翅膀。
    g.appendChild(pathOf(
      `M 0 0 C ${s * 14} -26 ${s * 40} -44 ${s * 54} -34 C ${s * 56} -14 ${s * 26} -2 0 0 Z`,
      { fill: `url(#${u}-wing)`, opacity: 0.84, stroke: `url(#${u}-rim)`, 'stroke-width': 1.1 },
    ))
    // 下翅：向下外的水滴形（裙摆两侧可见），与上翅之间留出明显缺口
    g.appendChild(pathOf(
      `M 0 3 C ${s * 12} 13 ${s * 30} 30 ${s * 24} 46 C ${s * 10} 44 ${s * 2} 24 0 3 Z`,
      { fill: `url(#${u}-wing)`, opacity: 0.7, stroke: `url(#${u}-rim)`, 'stroke-width': 1.1 },
    ))
    // 翅脉：让半透翼面读出结构（否则只是一块色斑）
    g.appendChild(pathOf(
      `M 0 0 C ${s * 18} -14 ${s * 34} -26 ${s * 48} -30`,
      { fill: 'none', stroke: `url(#${u}-rim)`, 'stroke-width': 1.1, opacity: 0.65 },
    ))
    g.appendChild(pathOf(
      `M 0 0 C ${s * 20} -6 ${s * 40} -12 ${s * 52} -20`,
      { fill: 'none', stroke: `url(#${u}-rim)`, 'stroke-width': 0.9, opacity: 0.45 },
    ))
    g.appendChild(pathOf(
      `M 2 8 C ${s * 10} 20 ${s * 18} 32 ${s * 22} 42`,
      { fill: 'none', stroke: `url(#${u}-rim)`, 'stroke-width': 0.9, opacity: 0.5 },
    ))
    g.setAttribute('data-side', side < 0 ? 'L' : 'R')
    return g
  }

  /** 精灵尖耳（细长上扬；锚点在头轮廓外侧，保证剪影可见） */
  private buildElfEar(side: -1 | 1): SVGGElement {
    const u = this.uid
    const p = this.palette
    const s = side
    const g = group({ transform: `translate(${s * 30} -2)` })
    g.appendChild(pathOf(
      `M 0 4 C ${s * 8} -14 ${s * 20} -26 ${s * 27} -24 C ${s * 21} -8 ${s * 11} 2 0 6 Z`,
      { fill: `url(#${u}-skin)`, stroke: p.skin.line, 'stroke-width': LW.minor },
    ))
    g.appendChild(pathOf(
      `M ${s * 5} 2 C ${s * 11} -9 ${s * 17} -17 ${s * 21} -17 C ${s * 16} -6 ${s * 9} 0 ${s * 5} 2 Z`,
      { fill: `url(#${u}-furAlt)`, opacity: 0.4 },
    ))
    g.setAttribute('data-side', side < 0 ? 'L' : 'R')
    return g
  }

  // ────────────────────────────────────────────────────────────────────
  // 逐帧：位姿 → SVG 变换
  // ────────────────────────────────────────────────────────────────────

  private loop = (): void => {
    if (this.disposed) return
    const nowMs = now()
    const dt = Math.min(0.05, (nowMs - this.lastT) / 1000)
    this.lastT = nowMs
    this.tick(dt, nowMs)
    this.raf = requestAnimationFrame(this.loop)
  }

  private tick(dt: number, nowMs: number): void {
    const t = (nowMs - this.t0) / 1000
    const mo = this.spec.motion

    // 眨眼调度（种子随机，可复现）
    if (nowMs / 1000 >= this.blinkNextAt) {
      this.blinkUntil = nowMs + 110
      this.blinkNextAt = nowMs / 1000 + mo.blink.baseSeconds + this.rand() * mo.blink.jitterSeconds
      if (!this.doubleBlinkPending && this.rand() < mo.blink.doubleBlinkProb) {
        this.doubleBlinkPending = true
      }
    }
    if (this.doubleBlinkPending && nowMs >= this.blinkUntil + 90) {
      this.blinkUntil = nowMs + 110
      this.doubleBlinkPending = false
    }
    this.blinkScaleY = nowMs < this.blinkUntil ? 0.06 : 1

    // 耳抽动调度
    if (nowMs / 1000 >= this.twitchNextAt) {
      this.twitchUntil = nowMs + 420
      this.twitchNextAt = nowMs / 1000 + mo.earTwitch.everySeconds + this.rand() * 2
    }
    const twitchPhase = nowMs < this.twitchUntil ? 1 : 0

    // 跳跃物理（落地下压 → squash，复起拉伸）
    const jumpT = (nowMs - this.jumpStart) / 1000
    if (this.jumpStart > 0 && jumpT < JUMP_DURATION) {
      this.jumpY = Math.max(0, this.jumpV0 * jumpT - 0.5 * JUMP_G * jumpT * jumpT)
      this.squash = 1 - Math.min(0.18, this.jumpY * 0.06)
    } else if (this.jumpStart > 0) {
      this.jumpStart = 0
      this.jumpY = 0
      this.squash = 1
    }

    // 行为推进
    if (this.dancingUntil > 0 && nowMs >= this.dancingUntil) this.dancingUntil = 0
    if (this.spinningUntil > 0 && nowMs >= this.spinningUntil) this.spinningUntil = 0
    this.behaviour = this.spinningUntil > 0 ? 'spin'
      : this.dancingUntil > 0 ? 'dance'
        : this.walking ? 'walk' : 'idle'
    // 行为态落到 data-* 上：CSS（如跳舞时的额外光效）与测试都据此观察，无需再猜内部状态
    if (this.behaviour !== this.lastBehaviour) {
      this.lastBehaviour = this.behaviour
      this.svg.setAttribute('data-behaviour', this.behaviour)
      this.svg.setAttribute('data-mood', this.mood)
    }
    if (this.walking) this.walkPhase += dt * 9

    // 悬停聚焦权重缓动
    this.hoverWeight += (this.hoverTarget - this.hoverWeight) * Math.min(1, dt * 6)

    // 注视：指针在容器内 → 跟随；否则缓慢左右张望
    const gaze = this.pointer.inside
      ? { x: this.pointer.x, y: this.pointer.y }
      : { x: Math.sin(t * 0.5) * 0.35, y: Math.sin(t * 0.31 + 1) * 0.18 }

    const pokeElapsed = this.pokeRegion ? (nowMs - this.pokeAt) / 1000 : null
    if (pokeElapsed !== null && pokeElapsed > 1.2) this.pokeRegion = null

    const input: PoseInput = {
      mood: this.mood,
      t,
      arousal: arousalOf(this.mood, this.hoverWeight),
      gazeX: gaze.x,
      gazeY: gaze.y,
      blinkScaleY: this.blinkScaleY,
      twitchPhase,
      jumpY: this.jumpY,
      squash: this.squash,
      isWalking: this.walking,
      walkPhase: this.walkPhase,
      isDancing: this.dancingUntil > 0,
      isSpinning: this.spinningUntil > 0,
      isBouncing: this.mood === 'happy' || this.mood === 'excited',
      isRubbingEyes: this.mood === 'sleepy',
      poke: this.pokeRegion ? { region: this.pokeRegion, elapsed: pokeElapsed ?? 0 } : null,
      focus: this.hoverWeight,
    }
    this.applyPose(computePose(this.spec, input), nowMs)
  }

  /** 位姿 → 变换（唯一改动 DOM 的地方；每帧只写必要属性） */
  private applyPose(pose: Pose, nowMs: number): void {
    const p = this.parts
    const spin = this.spinningUntil > 0
      ? Math.sin(((nowMs - this.spinStart) / (this.spinningUntil - this.spinStart)) * Math.PI)
      : 0
    // 转身（spin）用横向压缩 + 侧移模拟 → 2D 里的「转了一圈」
    const spinScaleX = 1 - Math.abs(spin) * 0.58
    const face = this.facing
    const rootX = CX + pose.rootPosX * 46 * face
    const rootY = GROUND_Y + pose.rootPosY * 60
    p.root.setAttribute(
      'transform',
      `translate(${rootX.toFixed(2)} ${rootY.toFixed(2)}) ` +
      `rotate(${(pose.rootRotZ * DEG).toFixed(2)}) ` +
      `scale(${(spinScaleX * face).toFixed(3)} 1) ` +
      `translate(${(-CX).toFixed(2)} ${(-GROUND_Y).toFixed(2)})`,
    )

    // 身体：呼吸缩放（**必须保留 translate 基位**）
    // 曾写成 `translate(a) scale(s) translate(-a)`——那是「绕 a 缩放」的写法，只适用于
    // 子节点已画在父级绝对坐标里的情况；而身体子节点画的是**组内局部坐标**，于是 s≈1 时
    // 整段变换退化成恒等，身体/裙/腿/鞋被画到画布左上角（截图实证：头浮着、下半身消失）。
    const bodyBaseY = GROUND_Y - this.bodyBaseOffset()
    p.body.setAttribute('transform',
      `translate(${CX} ${bodyBaseY.toFixed(2)}) ` +
      `scale(${pose.bodyScaleX.toFixed(4)} ${pose.bodyScaleY.toFixed(4)})`)

    // 头：yaw → 五官横移 + 远侧部件反向偏移（视差）；pitch → 纵向压缩下移
    const yaw = pose.headRotY
    const headBaseY = this.headBaseY() + pose.bodyPosY * 8
    const faceShift = Math.sin(yaw) * 7.5
    p.head.setAttribute('transform',
      `translate(${(CX + faceShift * 0.35).toFixed(2)} ${headBaseY.toFixed(2)}) ` +
      `rotate(${(pose.headRotZ * DEG).toFixed(2)}) ` +
      `translate(0 ${(-pose.headRotX * 2.4).toFixed(2)}) ` +
      `scale(${(1 - Math.abs(yaw) * 0.08).toFixed(3)} ${(1 - pose.headRotX * 0.06).toFixed(3)})`)
    p.face.setAttribute('transform', `translate(${faceShift.toFixed(2)} ${(pose.headRotX * 1.1).toFixed(2)})`)
    // 远侧耳压缩、近侧耳展开（侧脸时两耳不再同样大）
    const earLScale = 1 - Math.max(0, yaw) * 0.22
    const earRScale = 1 - Math.max(0, -yaw) * 0.22
    p.earL.setAttribute('transform', `${this.earBaseTransform('L')} rotate(${(pose.earRotZL * DEG).toFixed(2)}) scale(${earLScale.toFixed(3)})`)
    p.earR.setAttribute('transform', `${this.earBaseTransform('R')} rotate(${(pose.earRotZR * DEG).toFixed(2)}) scale(${earRScale.toFixed(3)})`)

    // 眼：眨眼用眼睑高度；瞳孔随视线偏移并缩放
    const lidH = (1 - pose.eyeScaleYL) * 26
    p.lidL.setAttribute('height', lidH.toFixed(2))
    p.lidR.setAttribute('height', ((1 - pose.eyeScaleYR) * 26).toFixed(2))
    p.lidL.setAttribute('y', (-13 - (1 - pose.eyeScaleYL) * 13).toFixed(2))
    p.lidR.setAttribute('y', (-13 - (1 - pose.eyeScaleYR) * 13).toFixed(2))
    const pupilDx = pose.pupilOffsetX * 130
    const pupilDy = pose.pupilOffsetY * 130
    const pupilScale = Math.min(1.35, Math.max(0.7, pose.pupilScale))
    for (const g of [p.pupilL, p.pupilR]) {
      g.setAttribute('transform', `translate(${pupilDx.toFixed(2)} ${pupilDy.toFixed(2)}) scale(${pupilScale.toFixed(3)})`)
    }
    p.eyeL.setAttribute('transform', `${this.eyeBaseTransform('L')} rotate(${(pose.eyeRotZL * DEG).toFixed(2)})`)
    p.eyeR.setAttribute('transform', `${this.eyeBaseTransform('R')} rotate(${(pose.eyeRotZR * DEG).toFixed(2)})`)

    // 眉 / 腮红 / 嘴
    p.browL.setAttribute('transform', `${this.browBaseTransform('L')} translate(0 ${(-pose.browPosY * 6).toFixed(2)}) rotate(${(pose.browRotZL * DEG).toFixed(2)})`)
    p.browR.setAttribute('transform', `${this.browBaseTransform('R')} translate(0 ${(-pose.browPosY * 6).toFixed(2)}) rotate(${(pose.browRotZR * DEG).toFixed(2)})`)
    p.blush.setAttribute('opacity', pose.blushOpacity.toFixed(3))
    showOnly(p.mouth, pose.mouthShape)
    if (pose.mouthShape === 'open') {
      p.mouth.open.setAttribute('transform', `scale(${(1 + pose.mouthOpen * 0.22).toFixed(3)} ${(1 + pose.mouthOpen * 0.5).toFixed(3)})`)
    }

    // 翅膀 / 尾 / 脚 / 头饰 / 光环
    if (this.spec.kind === 'humanoid') {
      const flap = 1 + pose.wingFlap * 0.12
      p.wingL.setAttribute('transform', `${this.wingBaseTransform('L')} scale(${(1 / flap).toFixed(3)} ${flap.toFixed(3)})`)
      p.wingR.setAttribute('transform', `${this.wingBaseTransform('R')} scale(${(1 / flap).toFixed(3)} ${flap.toFixed(3)})`)
    } else {
      // 兽形：翅膀不作主体，仅用翅膀数学驱动尾/耳的小幅联动（保持同一套位姿语义）
      p.wingL.setAttribute('transform', `translate(${CX} ${GROUND_Y})`)
      p.wingR.setAttribute('transform', `translate(${CX} ${GROUND_Y})`)
    }
    if (p.tail) {
      p.tail.setAttribute('transform',
        `translate(${CX + 30 * this.facing} ${GROUND_Y - 30}) rotate(${(pose.tailWag * 22).toFixed(2)}) ` +
        `translate(0 ${(-pose.tailChainWave * 8).toFixed(2)})`)
    }
    p.footL.setAttribute('transform', `${this.footBaseTransform('L')} translate(0 ${(pose.footPosYL * 26).toFixed(2)})`)
    p.footR.setAttribute('transform', `${this.footBaseTransform('R')} translate(0 ${(pose.footPosYR * 26).toFixed(2)})`)
    p.armL?.setAttribute('transform', `${this.armBaseTransform('L')} translate(0 ${(-pose.armLift * 14).toFixed(2)}) scale(1 ${(1 - pose.armLift * 0.08).toFixed(3)})`)
    p.armR?.setAttribute('transform', `${this.armBaseTransform('R')} translate(0 ${(-pose.armLift * 6).toFixed(2)})`)
    p.ornament.setAttribute('transform', `${this.ornamentBaseTransform()} rotate(${(pose.flowerRotZ * DEG).toFixed(2)}) translate(0 ${(-pose.flowerPosY * 26).toFixed(2)})`)
    if (p.hairBack) {
      p.hairBack.setAttribute('transform', `${this.hairBaseTransform()} rotate(${(pose.tailWag * 3).toFixed(2)})`)
    }
    // 光环：硬环 + 柔光同步亮灭（柔光更淡更大，避免生硬光圈）
    p.halo.setAttribute('opacity', pose.haloOpacity.toFixed(3))
    p.halo.setAttribute('transform', `rotate(${(pose.haloRotZ * 20).toFixed(2)} ${CX} 34)`)
    p.haloGlow.setAttribute('opacity', haloGlowOpacity(pose.haloOpacity).toFixed(3))
    // 接触投影：跳得越高越小越淡（悬浮感的锚）
    const lift = clamp01(this.jumpY * 3)
    p.shadow.setAttribute('rx', (42 * (1 - lift * 0.35)).toFixed(2))
    p.shadow.setAttribute('opacity', (0.28 * (1 - lift * 0.55)).toFixed(3))
  }

  // 各部件基准变换（构建期定死，逐帧在此基础上叠加增量，避免两处几何漂移）
  private bodyBaseOffset(): number {
    return this.spec.kind === 'humanoid' ? 62 : 30
  }
  private headBaseY(): number {
    return this.spec.kind === 'humanoid' ? GROUND_Y - 110 : GROUND_Y - 92
  }
  private earBaseTransform(side: 'L' | 'R'): string {
    const s = side === 'L' ? -1 : 1
    // 与 buildCatEar / buildElfEar 的锚点一致（头局部坐标）
    return this.spec.kind === 'humanoid' ? `translate(${s * 30} -2)` : `translate(${s * 30} -32)`
  }
  private eyeBaseTransform(side: 'L' | 'R'): string {
    const a = this.anchors
    const x = side === 'L' ? a.pupilL.x * 100 : a.pupilR.x * 100
    const y = this.spec.kind === 'humanoid' ? 4 : 6
    // 局部坐标（父级 head 已带绝对位移）
    return `translate(${x.toFixed(2)} ${y})`
  }
  private browBaseTransform(side: 'L' | 'R'): string {
    const s = side === 'L' ? -1 : 1
    return this.spec.kind === 'humanoid' ? `translate(${s * 16} -17)` : `translate(${s * 19} -14)`
  }
  private footBaseTransform(side: 'L' | 'R'): string {
    const s = side === 'L' ? -1 : 1
    return this.spec.kind === 'humanoid' ? `translate(${s * 8} 56)` : `translate(${s * 20} 30)`
  }
  private armBaseTransform(side: 'L' | 'R'): string {
    const s = side === 'L' ? -1 : 1
    return `translate(${s * 13} -4)`
  }
  private wingBaseTransform(side: 'L' | 'R'): string {
    const s = side === 'L' ? -1 : 1
    return `translate(${CX + s * 14} ${GROUND_Y - 120})`
  }
  private ornamentBaseTransform(): string {
    // 头部局部坐标（父级 head 已带绝对位移）
    return 'translate(0 -26)'
  }
  private hairBaseTransform(): string {
    return `translate(${CX} ${GROUND_Y - 108})`
  }

  /** 特效层容器（粒子/环境特效挂这里） */
  private parts_fx(): SVGGElement {
    return group({ class: 'pet-svg-fx' })
  }

  // ────────────────────────────────────────────────────────────────────
  // 公开 API（与旧 Pet3D 同名同义，宿主一行替换）
  // ────────────────────────────────────────────────────────────────────

  setMood(mood: PetMood3D): void {
    this.mood = mood
    this.svg.setAttribute('data-mood', mood)
  }

  /** 心情动作：进入心情时触发一次性动作（happy/excited 蹦跳，sleepy 揉眼由 pose 持续驱动） */
  triggerMoodAction(mood: PetMood3D): void {
    this.setMood(mood)
    if (mood === 'happy' || mood === 'excited') this.jump(1.15)
    if (mood === 'sleepy') this.jump(0.5)
  }

  setWalking(walking: boolean): void {
    this.walking = walking
  }

  setFacing(dir: 1 | -1): void {
    this.facing = dir
  }

  /** 转圈：2D 用横向压缩 + 圆周转体模拟（时长与旧版一致） */
  spin(duration = 0.9): void {
    const ms = duration * 1000
    this.spinStart = now()
    this.spinningUntil = this.spinStart + ms
    this.jump(1.6)
  }

  dance(duration = 3): void {
    this.dancingUntil = now() + duration * 1000
  }

  /** 部位点击：头=点头 / 背=侧倾 / 尾=甩尾（数学在 petPose，这里只置冲量） */
  poke(region: PetHitRegion): void {
    this.pokeRegion = region
    this.pokeAt = now()
    if (region === 'head') this.jump(0.85)
  }

  /** 悬停聚焦：0..1（光标停在它身上 → 更专注地看你 + 镜头感的轻微前推） */
  setHover(weight: number): void {
    this.hoverTarget = clamp01(weight)
  }

  /** 指针位置（容器归一化 -1..1）：驱动视线跟随 */
  setPointer(x: number, y: number, inside: boolean): void {
    this.pointer = { x: Math.max(-1, Math.min(1, x)), y: Math.max(-1, Math.min(1, y)), inside }
  }

  /** 视线跟随（与旧 Pet3D 同名）：等价于「指针在容器内」的 setPointer */
  lookAt(x: number, y: number): void {
    this.setPointer(x, y, true)
  }

  /** 一次性起跳（`strength` 越大跳得越高；与旧 Pet3D 同名同义） */
  jump(strength = 1): void {
    this.jumpStart = now()
    this.jumpV0 = strength
  }

  /**
   * 粒子爆发（庆祝/跳跃/抚摸）。
   *
   * 用 Web Animations API 播完即 `remove()`：不常驻 DOM、不无界增长；预算取自
   * `PET_PARTICLES` 与渲染档位（低档减半），与旧 WebGL 池同口径。
   */
  burst(color?: number, count: number = PET_PARTICLES.burstCount): void {
    const budget = PET_QUALITY[this.tier].particleBudget
    const n = Math.max(1, Math.round(count * budget))
    const tone = color ?? this.spec.accent
    for (let i = 0; i < n; i++) {
      const a = this.rand() * Math.PI * 2
      const r = 10 + this.rand() * 18
      const spark = this.burstShape(tone, i)
      spark.setAttribute('transform', `translate(${CX + Math.cos(a) * r * 0.5} ${GROUND_Y - 70 + Math.sin(a) * r * 0.5}) scale(${(0.5 + this.rand() * 0.5).toFixed(2)})`)
      this.parts.fx.appendChild(spark)
      const dx = Math.cos(a) * (26 + this.rand() * 26)
      const dy = Math.sin(a) * (16 + this.rand() * 18) - 26
      const dur = 520 + this.rand() * 380
      const anim = spark.animate(
        [
          { transform: `translate(${CX} ${GROUND_Y - 70}) scale(0.2)`, opacity: 0 },
          { transform: `translate(${CX + dx * 0.45} ${GROUND_Y - 70 + dy * 0.45}) scale(1)`, opacity: 1, offset: 0.32 },
          { transform: `translate(${CX + dx} ${GROUND_Y - 70 + dy}) scale(0.35)`, opacity: 0 },
        ],
        { duration: dur, easing: 'cubic-bezier(.22,.61,.36,1)', delay: i * 12 },
      )
      // 播完即摘除：特效不常驻 DOM（不留残影）
      anim.onfinish = () => spark.remove()
    }
  }

  /** 粒子形状：交替「星 / 心 / 花」，比单一点更生动 */
  private burstShape(tone: number, i: number): SVGElement {
    const kind = i % 3
    const fill = hexOf(shadeRgb(tone, kind === 1 ? 0.3 : 0))
    if (kind === 0) {
      return pathOf('M 0 -4 L 1.1 -1.1 L 4 0 L 1.1 1.1 L 0 4 L -1.1 1.1 L -4 0 L -1.1 -1.1 Z', { fill })
    }
    if (kind === 1) {
      return pathOf('M 0 3 C -4 -1 -3 -5 0 -3 C 3 -5 4 -1 0 3 Z', { fill })
    }
    return ellipse(0, 0, 3.2, 2.2, { fill, opacity: 0.9 })
  }

  /** 销毁：停 rAF、摘 DOM（宿主切换皮肤/卸载时调用） */
  dispose(): void {
    this.disposed = true
    if (this.raf) cancelAnimationFrame(this.raf)
    this.raf = 0
    this.svg.remove()
  }
}

// ──────────────────────────────────────────────────────────────────────
// 内部工具
// ──────────────────────────────────────────────────────────────────────

/** 跳跃时长与重力（归一化单位/秒；与 squash 联动） */
const JUMP_DURATION = 0.85
const JUMP_G = 9.8

/** 心情 → 唤醒度（调制尾频/耳幅/呼吸；与 petPose 的 arousalMod 同向） */
function arousalOf(mood: PetMood3D, focus: number): number {
  const base: Record<PetMood3D, number> = {
    excited: 0.95, happy: 0.8, working: 0.6, normal: 0.42, sad: 0.26, sleepy: 0.12,
  }
  return clamp01((base[mood] ?? 0.42) + focus * 0.18)
}

/** 只显示指定形态的嘴（其余 opacity=0，避免叠加成怪表情） */
function showOnly(
  mouths: Record<'smile' | 'open' | 'frown' | 'swoon', SVGGElement>,
  shape: Pose['mouthShape'],
): void {
  for (const [key, node] of Object.entries(mouths)) {
    node.setAttribute('opacity', key === shape ? '1' : '0')
  }
}

function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

/** 稳定的 FNV-1a 32 位哈希（渐变 id 去重；同皮肤同 id，跨实例不冲突） */
function hash32(s: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h | 0
}

/** 便捷工厂（与旧 `new Pet3D(opts)` 对称，便于宿主迁移与测试注入） */
export function createPetSvg(opts: PetSvgOptions): PetSvg {
  return new PetSvg(opts)
}

/** 皮肤 → 规格（供宿主/测试断言「渲染器拿到的确实是该皮肤」） */
export function resolvePetSvgSpec(skinId: string): PetModelSpec {
  return getPetModelSpec(skinId)
}