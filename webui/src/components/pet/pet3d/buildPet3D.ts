/**
 * 桌宠 3D 建模器（Three.js）—— 由 `PetModelSpec` 数据驱动，参数化生成真三维网格。
 *
 * 【为什么重新引入 3D】
 * 2026-10-23 的产品验收要求「3D 立体类桌宠支持多视角切换，各面渲染正常、无穿模、
 * 无纹理错误」。此前的 2.5D 矢量渲染在正面观感很好，但**没有背面**，无法满足
 * 「转到背面仍在建模」这条硬指标。旧版 `Pet3D.ts`（1848 行）已退役，本文件是
 * 按同一份 `petSpec` 数据重新实现的精简建模层：
 *   - 全部几何由 proportions / appearance 推导，没有硬编码的皮肤分支；
 *   - 材质统一走 `PET_TOON_RAMP` 的渐变量化（与既有卡通口径一致）；
 *   - 每个部件都有 `name`，便于单测断言「部件齐全」与多视角截图核对；
 *   - 只做几何与材质，不含舞台/相机/循环（见 `Pet3DScene.ts`），Node 环境可直接构造。
 *
 * 【安全边界】角色是 Q 版卡通造型：长裙、无裸露、无性化设计；不加载任何外部图片纹理
 * （材质全部是纯色 + 渐变量化），因此不存在「贴图内容不当」或「纹理加载失败」的风险。
 */

import * as THREE from 'three'
import {
  BEAST_EYE_ANCHORS,
  HUMANOID_EYE_ANCHORS,
  ORNAMENT_COLORS,
  PET_TOON_RAMP_WIDTH,
  eyeLayout,
  mergeAppearance,
  toonRampBytes,
  type PetModelSpec,
} from '../petSpec'

/** 可动部件引用（与 `Pose` 字段一一对应，逐帧只改变换/缩放/可见性）。 */
export interface Pet3DParts {
  root: THREE.Group
  /** 身体（呼吸/漂浮/跳跃的载体；缩放原点在脚底） */
  body: THREE.Group
  head: THREE.Group
  earL: THREE.Object3D
  earR: THREE.Object3D
  eyeL: THREE.Group
  eyeR: THREE.Group
  pupilL: THREE.Group
  pupilR: THREE.Group
  browL: THREE.Object3D
  browR: THREE.Object3D
  mouth: Record<'smile' | 'open' | 'frown' | 'swoon', THREE.Object3D>
  blush: THREE.Mesh[]
  wingL: THREE.Object3D | null
  wingR: THREE.Object3D | null
  tail: THREE.Object3D | null
  footL: THREE.Object3D
  footR: THREE.Object3D
  ornament: THREE.Object3D | null
  halo: THREE.Object3D | null
  hairBack: THREE.Object3D | null
  armL: THREE.Object3D | null
  armR: THREE.Object3D | null
  dress: THREE.Object3D | null
  shadow: THREE.Mesh
  /** 模型总高（世界单位）——相机取景用 */
  height: number
  /** 头部中心的世界高度 */
  headY: number
  kind: PetModelSpec['kind']
}

let rampTexture: THREE.DataTexture | null = null

/** 卡通渐变贴图（单例）：RGBA 字节布局由 `toonRampBytes()` 决定，避免 RGBFormat 陷阱。 */
export function toonGradientMap(): THREE.DataTexture {
  if (rampTexture) return rampTexture
  const bytes = new Uint8Array(toonRampBytes())
  const tex = new THREE.DataTexture(bytes, PET_TOON_RAMP_WIDTH, 1, THREE.RGBAFormat)
  tex.minFilter = THREE.NearestFilter
  tex.magFilter = THREE.NearestFilter
  tex.generateMipmaps = false
  tex.needsUpdate = true
  rampTexture = tex
  return tex
}

export interface ToonOptions {
  opacity?: number
  transparent?: boolean
  side?: THREE.Side
  emissive?: number
  emissiveIntensity?: number
}

export function toonMaterial(color: number, opts: ToonOptions = {}): THREE.MeshToonMaterial {
  return new THREE.MeshToonMaterial({
    color,
    gradientMap: toonGradientMap(),
    transparent: opts.transparent ?? false,
    opacity: opts.opacity ?? 1,
    side: opts.side ?? THREE.FrontSide,
    emissive: opts.emissive ?? 0x000000,
    emissiveIntensity: opts.emissiveIntensity ?? 0,
  })
}

function mesh(
  geometry: THREE.BufferGeometry,
  material: THREE.Material,
  name: string,
): THREE.Mesh {
  const m = new THREE.Mesh(geometry, material)
  m.name = name
  m.castShadow = false
  m.receiveShadow = false
  return m
}

const SPHERE = new THREE.SphereGeometry(1, 32, 24)
// 全模型共用一份单位球：`disposePet3D` 必须跳过它，否则销毁一只宠会把其它模型的几何一起废掉。
SPHERE.userData.shared = true

/** 复用单位球，靠 scale 变形（省顶点、保证圆润） */
function ball(
  material: THREE.Material,
  name: string,
  sx: number,
  sy: number,
  sz: number,
  x = 0,
  y = 0,
  z = 0,
): THREE.Mesh {
  const m = mesh(SPHERE, material, name)
  m.scale.set(sx, sy, sz)
  m.position.set(x, y, z)
  return m
}

/** 眼部四层（眼白/虹膜/瞳孔/高光）——锚点与半径全部来自 `eyeLayout` 单一事实源 */
function buildEye(
  side: 'L' | 'R',
  headRadius: number,
  eyeScale: number,
  anchors: { x: number; y: number },
  irisColor: number,
): { eye: THREE.Group; pupil: THREE.Group; baseZ: number } {
  const layout = eyeLayout(headRadius, eyeScale)
  const eye = new THREE.Group()
  eye.name = `eye${side}`
  eye.position.set(anchors.x, anchors.y, 0)

  const white = ball(toonMaterial(0xffffff), `eyeWhite${side}`, layout.whiteR, layout.whiteR, layout.whiteR * 0.72, 0, 0, layout.eyeZ)
  eye.add(white)

  const iris = ball(toonMaterial(irisColor), `iris${side}`, layout.irisR, layout.irisR, layout.irisR * 0.55, 0, 0, layout.irisZ)
  eye.add(iris)

  // 瞳孔必须压在虹膜**前面**：按 layout.pupilZ 摆放时，虹膜的前缘（irisZ + 0.55·irisR）
  // 会盖住瞳孔（实测正面只剩一块纯色虹膜，看不到瞳孔与高光）。
  const pupilZ = layout.irisZ + layout.irisR * 0.55 + layout.pupilR * 0.3 + 0.004
  const pupil = new THREE.Group()
  pupil.name = `pupil${side}`
  pupil.position.set(0, 0, pupilZ)
  const pupilMesh = ball(toonMaterial(0x2b2f3a), `pupilCore${side}`, layout.pupilR, layout.pupilR * 1.12, layout.pupilR * 0.35)
  pupil.add(pupilMesh)
  const shine = ball(new THREE.MeshBasicMaterial({ color: 0xffffff }), `shine${side}`, layout.shineR, layout.shineR, layout.shineR * 0.4, layout.shineR * 0.6, layout.shineR * 0.8, layout.pupilR * 0.34)
  pupil.add(shine)
  eye.add(pupil)

  return { eye, pupil, baseZ: layout.eyeZ }
}

/** 嘴：四种形态各建一份，运行时按 `Pose.mouthShape` 切可见性（无几何重建）。 */
function buildMouth(y: number, z: number, scale = 1): Record<'smile' | 'open' | 'frown' | 'swoon', THREE.Object3D> {
  // 颜色比旧版更柔和、尺寸更小：真机正面截图里 0.075 半径 + 深色会糊成一团「黑痣」。
  const color = 0x9a5563
  const smileGeo = new THREE.TorusGeometry(0.052 * scale, 0.010 * scale, 8, 20, Math.PI * 0.85)
  const smile = mesh(smileGeo, toonMaterial(color), 'mouth-smile')
  smile.position.set(0, y, z)
  smile.rotation.z = Math.PI + 0.12

  const frown = mesh(smileGeo.clone(), toonMaterial(color), 'mouth-frown')
  frown.position.set(0, y - 0.02, z)
  frown.rotation.z = -0.12

  const open = ball(toonMaterial(color), 'mouth-open', 0.04 * scale, 0.052 * scale, 0.022 * scale, 0, y - 0.01, z)
  const swoon = mesh(new THREE.TorusGeometry(0.045 * scale, 0.009 * scale, 8, 20, Math.PI), toonMaterial(color), 'mouth-swoon')
  swoon.position.set(0, y, z)
  swoon.rotation.z = Math.PI

  for (const m of [smile, frown, open, swoon]) m.visible = false
  smile.visible = true
  return { smile, open, frown, swoon }
}

function buildBlush(x: number, y: number, z: number, r: number): THREE.Mesh {
  const mat = new THREE.MeshBasicMaterial({ color: 0xff9db0, transparent: true, opacity: 0.35, depthWrite: false })
  const m = mesh(new THREE.CircleGeometry(r, 24), mat, 'blush')
  m.position.set(x, y, z)
  return m
}

function buildShadow(radius: number): THREE.Mesh {
  const mat = new THREE.MeshBasicMaterial({ color: 0x000000, transparent: true, opacity: 0.16, depthWrite: false })
  const m = mesh(new THREE.CircleGeometry(radius, 40), mat, 'shadow')
  m.rotation.x = -Math.PI / 2
  m.position.y = 0.004
  return m
}

/** 蝶翼/蝉翼形状（Bezier 花瓣），`flip` 控制左右镜像。 */
function wingGeometry(length: number, width: number): THREE.ShapeGeometry {
  const s = new THREE.Shape()
  s.moveTo(0, 0)
  s.bezierCurveTo(length * 0.25, width * 0.95, length * 0.82, width * 0.78, length, width * 0.12)
  s.bezierCurveTo(length * 0.86, -width * 0.18, length * 0.42, -width * 0.34, 0, -width * 0.06)
  s.closePath()
  const geo = new THREE.ShapeGeometry(s, 18)
  geo.computeVertexNormals()
  return geo
}

/** 花冠/花束共用：一片花瓣球 */
function petal(color: number, name: string, r = 0.055): THREE.Mesh {
  return ball(toonMaterial(color), name, r, r, r * 0.55)
}

/* ────────────────────────────── 兽形（云绒猫） ────────────────────────────── */

function buildBeast(spec: PetModelSpec): Pet3DParts {
  const app = mergeAppearance(spec)
  const p = spec.proportions
  const R = p.bodyRadius
  const HR = p.headRadius
  const bodyScaleY = p.bodyScaleY
  const pawH = 0.10
  const bodyCY = pawH + R * bodyScaleY
  const headCY = pawH + 2 * R * bodyScaleY + HR * 0.42
  const height = headCY + HR + p.earHeight

  const root = new THREE.Group()
  root.name = 'pet3d-root'
  const body = new THREE.Group()
  body.name = 'body'
  root.add(body)

  // 躯干 + 肚皮
  body.add(ball(toonMaterial(app.furColor), 'torso', R, R * bodyScaleY, R * 0.96, 0, bodyCY, 0))
  if (p.belly) {
    body.add(ball(toonMaterial(0xffffff), 'belly', R * 0.66, R * 0.58, R * 0.3, 0, bodyCY - R * 0.10, R * 0.66))
  }
  // 四只爪：前爪挂到 foot 组（脚部动画才有效果），后爪固定
  const footL = new THREE.Group()
  footL.name = 'footL'
  footL.position.set(-0.26, pawH * 0.9, 0.24)
  const footR = new THREE.Group()
  footR.name = 'footR'
  footR.position.set(0.26, pawH * 0.9, 0.24)
  footL.add(ball(toonMaterial(app.furColor), 'pawFrontL', 0.12, 0.085, 0.15))
  footR.add(ball(toonMaterial(app.furColor), 'pawFrontR', 0.12, 0.085, 0.15))
  body.add(footL, footR)
  body.add(ball(toonMaterial(app.furColor), 'pawBackL', 0.12, 0.085, 0.15, -0.26, pawH * 0.9, -0.18))
  body.add(ball(toonMaterial(app.furColor), 'pawBackR', 0.12, 0.085, 0.15, 0.26, pawH * 0.9, -0.18))

  // 项圈 + 铃铛
  // 半径必须大于该高度处的身体截面（0.62R 会整圈埋进躯干，只剩铃铛悬在肚子上）。
  const collar = mesh(new THREE.TorusGeometry(R * 0.74, 0.038, 12, 40), toonMaterial(app.furAccent), 'collar')
  collar.rotation.x = Math.PI / 2
  collar.position.y = bodyCY + R * bodyScaleY * 0.66
  collar.scale.z = 0.85
  body.add(collar)
  const bell = ball(toonMaterial(0xf4d06f, { emissive: 0x6b4f00, emissiveIntensity: 0.15 }), 'bell', 0.075, 0.075, 0.075, 0, collar.position.y - 0.055, R * 0.95)
  body.add(bell)

  // 头
  const head = new THREE.Group()
  head.name = 'head'
  head.position.set(0, headCY, 0)
  body.add(head)
  head.add(ball(toonMaterial(app.furColor), 'headBall', HR * 1.02, HR, HR * 0.94))

  // 耳（外锥 + 内耳）
  const earGeo = new THREE.ConeGeometry(HR * 0.36, p.earHeight * 1.25, 24)
  const innerEarGeo = new THREE.ConeGeometry(HR * 0.22, p.earHeight * 0.9, 20)
  const earL = new THREE.Group()
  earL.name = 'earL'
  // 耳根抬高到头顶附近（0.80×HR）：按 0.62 摆放时锥体整根埋在头球里，正面看不到耳朵。
  earL.position.set(-HR * 0.52, HR * 0.80, -0.02)
  earL.rotation.z = 0.24
  earL.add(mesh(earGeo, toonMaterial(app.furColor), 'earOuterL'))
  const innerL = mesh(innerEarGeo, toonMaterial(0xffc9d4), 'earInnerL')
  innerL.position.set(0.005, 0.01, 0.055)
  earL.add(innerL)
  const earR = earL.clone(true)
  earR.name = 'earR'
  earR.position.x = HR * 0.52
  earR.rotation.z = -0.24
  head.add(earL, earR)

  // 眼
  const irisColor = spec.accent
  const L = buildEye('L', HR, p.eyeScale, { x: BEAST_EYE_ANCHORS.eyeL.x, y: BEAST_EYE_ANCHORS.eyeL.y }, irisColor)
  const Rgt = buildEye('R', HR, p.eyeScale, { x: BEAST_EYE_ANCHORS.eyeR.x, y: BEAST_EYE_ANCHORS.eyeR.y }, irisColor)
  head.add(L.eye, Rgt.eye)

  // 眉（小长方体，微表情用）—— 尺寸压到 0.10×0.022：旧尺寸在真机正面像两道「怒眉」
  const browGeo = new THREE.BoxGeometry(0.10, 0.022, 0.024)
  const browL = mesh(browGeo, toonMaterial(app.furAccent), 'browL')
  browL.position.set(BEAST_EYE_ANCHORS.eyeL.x, BEAST_EYE_ANCHORS.eyeL.y + HR * 0.30, HR * 0.80)
  browL.rotation.z = 0.10
  const browR = mesh(browGeo.clone(), toonMaterial(app.furAccent), 'browR')
  browR.position.set(BEAST_EYE_ANCHORS.eyeR.x, BEAST_EYE_ANCHORS.eyeR.y + HR * 0.30, HR * 0.80)
  browR.rotation.z = -0.10
  head.add(browL, browR)

  // 口鼻
  head.add(ball(toonMaterial(0xffffff), 'muzzle', HR * 0.36, HR * 0.26, HR * 0.22, 0, -HR * 0.12, HR * 0.80))
  const nose = mesh(new THREE.ConeGeometry(0.042, 0.06, 20), toonMaterial(0xff9db0), 'nose')
  nose.rotation.x = Math.PI / 2
  // 鼻尖必须探出口鼻球表面（口鼻球前缘 ≈ 0.80HR + 0.22HR），否则被吞进白球里
  nose.position.set(0, -HR * 0.06, HR * 1.04)
  head.add(nose)
  // 嘴贴在头球表面之外（头球前缘在 y=-0.24HR 处 ≈ 0.91HR；取 0.95HR 保证可见）
  const mouth = buildMouth(-HR * 0.24, HR * 0.95, 0.7)
  for (const m of Object.values(mouth)) head.add(m)

  // 胡须
  const whiskerGeo = new THREE.CylinderGeometry(0.0045, 0.0045, 0.26, 6)
  for (const [i, [sx, sy, rz]] of ([[-1, 0.02, 0.18], [-1, -0.04, 0.02], [1, 0.02, -0.18], [1, -0.04, -0.02]] as const).entries()) {
    const w = mesh(whiskerGeo, new THREE.MeshBasicMaterial({ color: 0xb9c4d4 }), `whisker${i}`)
    w.position.set(sx * HR * 0.52, sy * HR, HR * 0.86)
    w.rotation.z = Math.PI / 2 + rz
    head.add(w)
  }

  // 腮红（贴面片：必须落在头球表面之外，否则被球体遮住）
  const blush = [buildBlush(-HR * 0.60, -HR * 0.16, HR * 0.78, HR * 0.13), buildBlush(HR * 0.60, -HR * 0.16, HR * 0.78, HR * 0.13)]
  for (const b of blush) head.add(b)

  // 尾巴（球尾 + 曲线管），基座在背部
  const tail = new THREE.Group()
  tail.name = 'tail'
  tail.position.set(0, bodyCY + R * 0.30, -R * 0.86)
  const curve = new THREE.CatmullRomCurve3([
    new THREE.Vector3(0, 0, 0),
    new THREE.Vector3(0, 0.12, -0.22),
    new THREE.Vector3(0.02, 0.36, -0.30),
    new THREE.Vector3(0.02, 0.52, -0.12),
  ])
  tail.add(mesh(new THREE.TubeGeometry(curve, 24, 0.055, 10, false), toonMaterial(app.furAccent), 'tailTube'))
  const puff = ball(toonMaterial(0xffffff), 'tailPuff', 0.14, 0.14, 0.14, 0.02, 0.54, -0.10)
  tail.add(puff)
  body.add(tail)

  const halo: THREE.Object3D | null = null
  const ornament: THREE.Object3D | null = null
  const wings: { wingL: THREE.Object3D | null; wingR: THREE.Object3D | null } = { wingL: null, wingR: null }

  const shadow = buildShadow(R * 1.15)
  // 影子由舞台单独挂到场景（不跟随跳跃抬升），因此不加入 root。

  // 猫耳不是「抽动耳」部件？保留同一接口，动画层照常驱动。
  return {
    root,
    body,
    head,
    earL,
    earR,
    eyeL: L.eye,
    eyeR: Rgt.eye,
    pupilL: L.pupil,
    pupilR: Rgt.pupil,
    browL,
    browR,
    mouth,
    blush,
    ...wings,
    tail,
    footL,
    footR,
    ornament,
    halo,
    hairBack: null,
    armL: null,
    armR: null,
    dress: null,
    shadow,
    height,
    headY: headCY,
    kind: 'beast',
  }
}

/* ────────────────────────────── 人形（Q 版精灵） ────────────────────────────── */

function buildHumanoid(spec: PetModelSpec): Pet3DParts {
  const app = mergeAppearance(spec)
  const p = spec.proportions
  const HR = p.headRadius
  const legLen = p.legLength ?? 0.58
  const torsoLen = p.torsoLength ?? 0.34
  const armLen = p.armLength ?? 0.42
  const shoeH = 0.09
  const hipY = shoeH + legLen
  const shoulderY = hipY + torsoLen
  const headCY = shoulderY + 0.10 + HR * 0.86
  const height = headCY + HR * 1.06 + 0.08

  const root = new THREE.Group()
  root.name = 'pet3d-root'
  const body = new THREE.Group()
  body.name = 'body'
  root.add(body)

  // 腿 + 芭蕾鞋
  const legGeo = new THREE.CapsuleGeometry(0.072, Math.max(0.05, legLen - 0.16), 8, 18)
  const legL = mesh(legGeo, toonMaterial(app.skin), 'legL')
  legL.position.set(-0.10, shoeH + legLen * 0.5, 0)
  const legR = mesh(legGeo.clone(), toonMaterial(app.skin), 'legR')
  legR.position.set(0.10, shoeH + legLen * 0.5, 0)
  body.add(legL, legR)

  const shoeGeo = new THREE.SphereGeometry(0.09, 24, 16)
  const shoeL = mesh(shoeGeo, toonMaterial(app.shoeColor), 'shoeL')
  shoeL.scale.set(1, 0.62, 1.5)
  shoeL.position.set(-0.10, shoeH * 0.7, 0.045)
  const shoeR = mesh(shoeGeo.clone(), toonMaterial(app.shoeColor), 'shoeR')
  shoeR.scale.set(1, 0.62, 1.5)
  shoeR.position.set(0.10, shoeH * 0.7, 0.045)
  body.add(shoeL, shoeR)

  const footL = new THREE.Group()
  footL.name = 'footL'
  footL.position.copy(shoeL.position)
  const footR = new THREE.Group()
  footR.name = 'footR'
  footR.position.copy(shoeR.position)
  body.add(footL, footR)

  // 花瓣嫁裙（主裙 + 下摆薄纱 + 腰带）
  const dress = new THREE.Group()
  dress.name = 'dress'
  const skirtTop = hipY + 0.26
  const skirtBottom = shoeH + 0.30
  const skirtH = skirtTop - skirtBottom
  const skirt = mesh(
    new THREE.CylinderGeometry(0.17, 0.46, skirtH, 48, 1, false),
    toonMaterial(app.dressColor),
    'dress-skirt',
  )
  skirt.position.y = (skirtTop + skirtBottom) / 2
  dress.add(skirt)
  const hem = mesh(new THREE.CylinderGeometry(0.40, 0.50, 0.14, 48, 1, true), toonMaterial(app.dressAccent, { side: THREE.DoubleSide }), 'dress-hem')
  hem.position.y = skirtBottom + 0.06
  dress.add(hem)
  const belt = mesh(new THREE.TorusGeometry(0.175, 0.022, 10, 40), toonMaterial(app.dressAccent), 'dress-belt')
  belt.rotation.x = Math.PI / 2
  belt.position.y = hipY + 0.24
  dress.add(belt)
  body.add(dress)

  // 上身
  const bodice = mesh(new THREE.CapsuleGeometry(0.135, 0.14, 8, 24), toonMaterial(app.dressColor), 'bodice')
  bodice.position.y = hipY + torsoLen * 0.5 + 0.06
  body.add(bodice)

  // 手臂（上抬由 `Pose.armLift` 驱动）
  const armGeo = new THREE.CapsuleGeometry(0.05, Math.max(0.06, armLen - 0.12), 8, 16)
  const makeArm = (side: 'L' | 'R'): THREE.Group => {
    const g = new THREE.Group()
    g.name = `arm${side}`
    const sign = side === 'L' ? -1 : 1
    g.position.set(sign * 0.19, shoulderY - 0.04, 0)
    g.rotation.z = sign * -0.34
    const upper = mesh(armGeo.clone(), toonMaterial(app.skin), `armUpper${side}`)
    upper.position.y = -armLen * 0.42
    g.add(upper)
    const hand = ball(toonMaterial(app.skin), `hand${side}`, 0.062, 0.062, 0.062, 0, -armLen * 0.86, 0)
    g.add(hand)
    return g
  }
  const armL = makeArm('L')
  const armR = makeArm('R')
  body.add(armL, armR)

  // 颈 + 头
  const neck = mesh(new THREE.CylinderGeometry(0.058, 0.062, 0.10, 20), toonMaterial(app.skin), 'neck')
  neck.position.y = shoulderY + 0.05
  body.add(neck)

  const head = new THREE.Group()
  head.name = 'head'
  head.position.set(0, headCY, 0)
  body.add(head)
  // 头球：z 半径 0.99（原来 0.92）。
  // 【2026-10-23 真机截图】眼部四层按 eyeZ=0.808·HR + 眼白深度 0.194·HR 推到 1.002·HR，
  // 而脸面前缘只有 0.92·HR ⇒ 侧视图里眼球整颗凸出轮廓外（穿模），腮红也悬在脸前 0.05·HR。
  // 把脸做圆（z≈1.0）后，眼睛落回脸面、腮红贴住脸颊。
  head.add(ball(toonMaterial(app.skin), 'headBall', HR, HR * 1.02, HR * 0.99))

  // 后发（大形）+ 长发（垂到腰）+ 侧鬓 + 齐刘海
  const hairBack = new THREE.Group()
  hairBack.name = 'hairBack'
  // 【2026-10-23 真机截图修正】原后发球只向后偏 0.12·HR，而它的 x/y 半径都 ≥ 头球，
  // 于是整颗头被「发罩」包住：正面只露出一小块脸、顶部多出一圈像帽檐的发缘
  // （侧视图里那块白色薄片就是它）。后移 0.40·HR 后，前面让位给脸（前表面 0.60·HR，
  // 远在脸面 0.92·HR 之内），顶部与后脑仍被完整覆盖，发际线交给刘海。
  // 半径收到 ≤1.02：原来是 1.04/1.05/1.0，头顶（1.07·HR）比发盖还高，
  // 两层几乎共面 → 真机特写里颅顶出现一圈锯齿状 z-fighting。现在发盖（1.08·HR）稳定压在它上面。
  const backMass = ball(toonMaterial(app.furColor), 'hairBackMass', HR * 1.02, HR * 1.0, HR * 1.0, 0, HR * 0.02, -HR * 0.40)
  hairBack.add(backMass)
  // 头顶发盖：半球壳（开口朝下），开口前缘略抬 —— 只盖住颅顶到发际线，
  // 让脸留出完整椭圆。没有这一层时，后发后移后会露出「光头」的颅顶（真机截图实测）。
  // 半径 1.08·HR（不是 1.03）：头球 y 半径 1.02·HR，两者只差 0.01 时前额上方整片共面，
  // 真机 2x 特写里出现锯齿状 z-fighting（白皮肤从发盖里"啃"出来的尖齿）。
  const hairCap = mesh(
    new THREE.SphereGeometry(HR * 1.08, 40, 22, 0, Math.PI * 2, 0, Math.PI * 0.44),
    toonMaterial(app.furColor, { side: THREE.DoubleSide }),
    'hairCap',
  )
  hairCap.position.set(0, HR * 0.02, -HR * 0.06)
  hairCap.rotation.x = -0.14 // 前缘抬高（弧度）：露出额头，侧缘仍压住鬓角
  hairBack.add(hairCap)
  const longHair = mesh(
    new THREE.CylinderGeometry(HR * 0.62, HR * 0.30, HR * 1.90, 32, 1, true),
    toonMaterial(app.furColor, { side: THREE.DoubleSide }),
    'hairLong',
  )
  longHair.scale.z = 0.58
  longHair.position.set(0, -HR * 1.02, -HR * 0.26)
  hairBack.add(longHair)
  const tip = mesh(new THREE.CylinderGeometry(HR * 0.30, HR * 0.02, HR * 0.5, 32, 1, true), toonMaterial(app.furAccent, { side: THREE.DoubleSide }), 'hairTip')
  tip.scale.z = 0.58
  tip.position.set(0, -HR * 2.05, -HR * 0.26)
  hairBack.add(tip)
  head.add(hairBack)

  // 鬓发（耳前的两缕）
  // 【2026-10-23 真机特写】原来用发梢色（比发盖深两档）、长 0.86·HR、贴着 x=0.92·HR，
  // 在发盖加厚到 1.08·HR 之后看起来像两根悬在脸侧的深色"铁管"。
  // 现在与主发同色、缩短到 0.70·HR、往后压一点，读作压在耳前的自然发缕。
  const lockGeo = new THREE.CapsuleGeometry(0.05, HR * 0.70, 8, 14)
  for (const side of [-1, 1] as const) {
    const lock = mesh(lockGeo, toonMaterial(app.furColor), `sideLock${side < 0 ? 'L' : 'R'}`)
    lock.position.set(side * HR * 0.94, -HR * 0.24, HR * 0.02)
    lock.rotation.z = side * 0.09
    head.add(lock)
  }
  // 刘海：**不再单独建**。
  // 【2026-10-23 真机特写】头顶发盖（hairCap）与 5 片刘海球是同色不同调的两套曲面，
  // 交叠处会在额头留下一圈锯齿状"蛋壳边"（花瓣带下方那排碎白片）。
  // 发际线现在由发盖前缘 + 花冠这条带子共同定义，正面已经没有可穿的接缝。

  // 精灵尖耳
  // 【2026-10-23 真机特写】发盖加厚到 1.08·HR 后，原来 x=0.92·HR 的耳朵整根埋进发里（正面看不到尖耳）。
  // 外移到 1.12·HR 并稍向前，让耳尖从发际线外侧探出来。
  const earGeo = new THREE.ConeGeometry(0.055, 0.28, 18)
  const earL = new THREE.Group()
  earL.name = 'earL'
  earL.position.set(-HR * 1.12, HR * 0.02, HR * 0.06)
  earL.rotation.z = 1.28
  earL.add(mesh(earGeo, toonMaterial(app.skin), 'earOuterL'))
  const earR = new THREE.Group()
  earR.name = 'earR'
  earR.position.set(HR * 1.12, HR * 0.02, HR * 0.06)
  earR.rotation.z = -1.28
  earR.add(mesh(earGeo.clone(), toonMaterial(app.skin), 'earOuterR'))
  head.add(earL, earR)

  // 眼
  const L = buildEye('L', HR, p.eyeScale, { x: HUMANOID_EYE_ANCHORS.eyeL.x, y: HUMANOID_EYE_ANCHORS.eyeL.y }, spec.accent)
  const Rgt = buildEye('R', HR, p.eyeScale, { x: HUMANOID_EYE_ANCHORS.eyeR.x, y: HUMANOID_EYE_ANCHORS.eyeR.y }, spec.accent)
  head.add(L.eye, Rgt.eye)

  const browGeo = new THREE.BoxGeometry(0.09, 0.018, 0.024)
  const browL = mesh(browGeo, toonMaterial(app.furAccent), 'browL')
  // z 必须贴住脸面（该高度的脸面 ≈0.97·HR）：原来 0.78·HR 埋在头球里，正面完全看不到眉毛。
  browL.position.set(HUMANOID_EYE_ANCHORS.eyeL.x, HUMANOID_EYE_ANCHORS.eyeL.y + HR * 0.34, HR * 0.93)
  browL.rotation.z = 0.05
  const browR = mesh(browGeo.clone(), toonMaterial(app.furAccent), 'browR')
  browR.position.set(HUMANOID_EYE_ANCHORS.eyeR.x, HUMANOID_EYE_ANCHORS.eyeR.y + HR * 0.34, HR * 0.93)
  browR.rotation.z = -0.05
  head.add(browL, browR)

  const mouth = buildMouth(-HR * 0.34, HR * 0.95, 0.62)
  for (const m of Object.values(mouth)) head.add(m)
  const blush = [buildBlush(-HR * 0.58, -HR * 0.20, HR * 0.80, HR * 0.12), buildBlush(HR * 0.58, -HR * 0.20, HR * 0.80, HR * 0.12)]
  for (const b of blush) head.add(b)

  // 花冠（花型随皮肤）
  let ornament: THREE.Object3D | null = null
  if (app.ornament !== 'none') {
    const colors = ORNAMENT_COLORS[app.ornament]
    const g = new THREE.Group()
    g.name = 'ornament'
    // 【2026-10-23 真机截图修正】花冠必须压在**刘海这条发带上**、沿头围外沿露出来：
    // 旧版半径 0.68·HR 落在头球内部（该高度头围 ≈0.83·HR），侧视图里只剩一条绿线
    // 横穿头发、白花瓣闷在发罩里像一顶帽子。高度降到 0.52·HR 是为了让正面那朵花
    // 落在淡紫刘海之上（白花瓣压白脸＝看不见），后脑那段则埋进长发。
    g.position.set(0, HR * 0.52, 0)
    g.rotation.x = -0.10
    const ring = mesh(new THREE.TorusGeometry(1, 0.011, 8, 56), toonMaterial(0x7fae7a), 'ornamentRing')
    ring.rotation.x = Math.PI / 2
    // x 贴住两侧发面（该高度的发面 ≈0.88·HR），z 略大于刘海最前沿（0.91·HR）——
    // 正面那朵花才是**贴在刘海上**、以淡紫发为底（白花瓣压白脸会看不见）。
    ring.scale.set(HR * 0.90, HR * 1.0, 1)
    g.add(ring)
    const petals = 7
    for (let i = 0; i < petals; i += 1) {
      const a = (i / petals) * Math.PI * 2
      const flower = new THREE.Group()
      flower.position.set(Math.cos(a) * HR * 0.90, 0, -Math.sin(a) * HR * 1.0)
      // 花头朝外（+Z 转到径向），否则所有花瓣都朝正前方贴成一片
      flower.rotation.y = a + Math.PI / 2
      for (let k = 0; k < 5; k += 1) {
        const pa = (k / 5) * Math.PI * 2
        flower.add(
          ball(
            toonMaterial(colors.petal),
            `petal${i}-${k}`,
            0.048,
            0.048,
            0.020,
            Math.cos(pa) * 0.050,
            Math.sin(pa) * 0.050,
            0,
          ),
        )
      }
      flower.add(ball(toonMaterial(colors.center), `pistil${i}`, 0.030, 0.030, 0.018, 0, 0, 0.018))
      g.add(flower)
    }
    ornament = g
    head.add(g)
  }

  // 蝶翼（4 片；双面材质，背面同样有面）
  let wingL: THREE.Object3D | null = null
  let wingR: THREE.Object3D | null = null
  if (app.wingStyle !== 'none') {
    const upperGeo = wingGeometry(0.70, 0.52)
    const lowerGeo = wingGeometry(0.46, 0.30)
    const wingMat = toonMaterial(app.dressAccent, {
      transparent: true,
      opacity: 0.82,
      side: THREE.DoubleSide,
      emissive: spec.accent,
      emissiveIntensity: 0.22,
    })
    const makeWings = (side: 1 | -1): THREE.Group => {
      const g = new THREE.Group()
      g.name = side < 0 ? 'wingL' : 'wingR'
      g.position.set(side * 0.20, shoulderY + 0.24, -0.24)
      g.rotation.y = side * 0.48
      const upper = mesh(upperGeo, wingMat, side < 0 ? 'wingUpperL' : 'wingUpperR')
      upper.scale.x = side
      upper.rotation.z = side * 0.42
      const lower = mesh(lowerGeo, wingMat, side < 0 ? 'wingLowerL' : 'wingLowerR')
      lower.scale.x = side
      lower.rotation.z = side * -0.48
      lower.position.set(0, -0.22, 0)
      g.add(upper, lower)
      return g
    }
    wingL = makeWings(-1)
    wingR = makeWings(1)
    body.add(wingL!, wingR!)
  }

  // 捧花
  if (app.hasBouquet) {
    const bouquet = new THREE.Group()
    bouquet.name = 'bouquet'
    // 放在右手外侧（手 ≈ x 0.28, y shoulderY-0.40）：贴着裙面会被当成一枚胸针。
    bouquet.position.set(0.34, shoulderY - 0.42, 0.24)
    bouquet.rotation.z = -0.22
    // 整体放大：按 1.0 建的花束在 380px 舞台里只有几个像素，看不出是「捧花」。
    bouquet.scale.setScalar(1.5)
    const colors = app.ornament !== 'none' ? ORNAMENT_COLORS[app.ornament] : ORNAMENT_COLORS.daisy
    for (let i = 0; i < 5; i += 1) {
      const a = (i / 5) * Math.PI * 2
      bouquet.add(petal(colors.petal, `bouquetPetal${i}`, 0.05).translateX(Math.cos(a) * 0.06).translateY(Math.sin(a) * 0.06))
    }
    bouquet.add(petal(colors.center, 'bouquetCenter', 0.038))
    for (let i = 0; i < 3; i += 1) {
      const stem = mesh(new THREE.CylinderGeometry(0.006, 0.006, 0.16, 6), toonMaterial(0x6f9e6a), `stem${i}`)
      stem.position.set((i - 1) * 0.02, -0.10, 0)
      stem.rotation.z = (i - 1) * 0.18
      bouquet.add(stem)
    }
    body.add(bouquet)
  }

  // 光环（working/excited 时亮起）
  const halo = new THREE.Group()
  halo.name = 'halo'
  halo.position.set(0, headCY + HR * 1.28, 0)
  const haloMesh = mesh(
    new THREE.TorusGeometry(HR * 0.52, 0.014, 8, 48),
    toonMaterial(spec.accent, { transparent: true, opacity: 0, emissive: spec.accent, emissiveIntensity: 0.9 }),
    'haloRing',
  )
  haloMesh.rotation.x = Math.PI / 2
  halo.add(haloMesh)
  body.add(halo)

  const shadow = buildShadow(0.52)
  // 影子由舞台单独挂到场景（不跟随跳跃抬升），因此不加入 root。

  return {
    root,
    body,
    head,
    earL,
    earR,
    eyeL: L.eye,
    eyeR: Rgt.eye,
    pupilL: L.pupil,
    pupilR: Rgt.pupil,
    browL,
    browR,
    mouth,
    blush,
    wingL,
    wingR,
    tail: null,
    footL,
    footR,
    ornament,
    halo,
    hairBack,
    armL,
    armR,
    dress,
    shadow,
    height,
    headY: headCY,
    kind: 'humanoid',
  }
}

/** 统一入口：按 `spec.kind` 建模。任何几何异常都会在构造期抛出，不静默降级。 */
export function buildPet3D(spec: PetModelSpec): Pet3DParts {
  const parts = spec.kind === 'humanoid' ? buildHumanoid(spec) : buildBeast(spec)
  const hs = spec.proportions.heightScale ?? 1
  if (Number.isFinite(hs) && hs > 0 && hs !== 1) {
    parts.root.scale.setScalar(hs)
    parts.height *= hs
    parts.headY *= hs
  }
  // 统一基调：所有部件都参与视锥剔除（避免背面被误裁剪）
  parts.root.traverse((o) => { o.frustumCulled = true })
  return parts
}

/** 释放本模型持有的 GPU 资源（几何/材质；渐变贴图与共享单位球不在此释放）。 */
export function disposePet3D(parts: Pet3DParts): void {
  const materials = new Set<THREE.Material>()
  const collect = (root: THREE.Object3D): void => {
    root.traverse((o) => {
      const m = o as THREE.Mesh
      if (m.isMesh) {
        if (!m.geometry?.userData?.shared) m.geometry?.dispose()
        const mat = m.material
        if (Array.isArray(mat)) mat.forEach((x) => materials.add(x))
        else if (mat) materials.add(mat as THREE.Material)
      }
    })
  }
  collect(parts.root)
  collect(parts.shadow)
  materials.forEach((mat) => mat.dispose())
}
