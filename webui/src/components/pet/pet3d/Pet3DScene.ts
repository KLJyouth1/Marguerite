/**
 * 桌宠 3D 舞台（Three.js）—— 建模之外的全部运行时：相机、灯光、环绕视角、动画循环。
 *
 * 公开 API 与 `PetSvg` 同名同义（setMood / triggerMoodAction / setPointer / lookAt /
 * jump / burst / dispose …），宿主可以在两种渲染器之间一行切换。额外提供：
 *   - `setView('front' | 'threeQuarter' | 'side' | 'back' | 'free')`：真三維多视角；
 *   - `setAutoRotate(true)`：自动环绕（演示/展示态）；
 *   - `orbitBy(dx, dy)`：拖拽环绕（由本类内部的指针事件直接驱动）。
 *
 * 多视角是「转相机」而不是「翻转模型」：光源固定在世界坐标，转到背面时看到的是
 * 真实的背部几何（后发/裙背/翼背面），不会出现镜像贴图或穿模。
 *
 * 诊断：`container.dataset.pet3d*` 写入就绪/视角/方位角，供 e2e 断言（不依赖像素比对）。
 */

import * as THREE from 'three'
import { computePose, type PetHitRegion, type PoseInput } from '../petPose'
import {
  PET_CAMERA,
  PET_LIGHTING,
  cameraZoomTarget,
  stepCameraZoom,
} from '../petSpec'
import type { PetMood3D, PetRenderTier } from '../petContract'
import type { PetModelSpec } from '../petSpec'
import { buildPet3D, disposePet3D, type Pet3DParts } from './buildPet3D'
import {
  PET_3D_POLAR_BASE,
  clampPolar,
  stepAngle,
  viewAzimuth,
  normalizeAzimuth,
  type Pet3DView,
} from './views'

export interface Pet3DOptions {
  container: HTMLElement
  spec: PetModelSpec
  accent?: number
  quality?: PetRenderTier
}

interface Burst {
  points: THREE.Points
  velocities: Float32Array
  bornAt: number
  life: number
}

const TWO_PI = Math.PI * 2

function arousalOf(mood: PetMood3D, hover: number): number {
  const base = mood === 'excited' ? 1 : mood === 'happy' ? 0.86 : mood === 'working' ? 0.7
    : mood === 'normal' ? 0.5 : mood === 'sad' ? 0.3 : 0.16
  return Math.min(1, base + hover * 0.25)
}

export class Pet3DScene {
  private readonly container: HTMLElement
  private readonly spec: PetModelSpec
  private readonly renderer: THREE.WebGLRenderer
  private readonly scene: THREE.Scene
  private readonly camera: THREE.PerspectiveCamera
  private readonly parts: Pet3DParts
  private readonly keyLight: THREE.DirectionalLight
  private readonly rimLight: THREE.PointLight
  private readonly resizeObserver: ResizeObserver | null = null
  private readonly onWindowResize: (() => void) | null = null

  private mood: PetMood3D = 'normal'
  private hoverTarget = 0
  private hoverWeight = 0
  private pointer = { x: 0, y: 0, inside: false }
  private blinkNextAt = 1.6
  private blinkUntil = 0
  private doubleBlink = false
  private twitchNextAt = 3.5
  private twitchUntil = 0
  private jumpY = 0
  private jumpV = 0
  private squash = 1
  private dancingUntil = 0
  private spinningUntil = 0
  private spinStart = 0
  private pokeRegion: PetHitRegion | null = null
  private pokeAt = 0
  private facingYaw = 0
  /** 散步步态（宿主「散步」行为驱动） */
  private walking = false

  private azimuth = 0
  private polar = PET_3D_POLAR_BASE
  private targetAzimuth: number | null = 0
  private zoom = 1
  private zoomTarget = 1
  private autoRotate = false
  private view: Pet3DView = 'front'

  private raf = 0
  private lastMs = 0
  private t0 = 0
  private disposed = false
  private readonly rand: () => number
  private bursts: Burst[] = []

  // 指针拖拽
  private dragging = false
  private dragStart = { x: 0, y: 0, azimuth: 0, polar: 0 }
  private dragMoved = 0
  private readonly onPointerDown: (e: PointerEvent) => void
  private readonly onPointerMove: (e: PointerEvent) => void
  private readonly onPointerUp: (e: PointerEvent) => void
  private readonly onWheel: (e: WheelEvent) => void
  private readonly onPointerLeave: () => void

  constructor(opts: Pet3DOptions) {
    this.container = opts.container
    this.spec = opts.spec
    this.rand = mulberry32(hashString(this.spec.id))

    const width = Math.max(1, this.container.clientWidth || 640)
    const height = Math.max(1, this.container.clientHeight || 380)
    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'high-performance' })
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1))
    this.renderer.setSize(width, height, false)
    this.renderer.setClearColor(0x000000, 0)
    const canvas = this.renderer.domElement
    canvas.className = 'pet-3d-canvas'
    canvas.style.width = '100%'
    canvas.style.height = '100%'
    canvas.style.display = 'block'
    canvas.setAttribute('role', 'img')
    canvas.setAttribute('aria-label', `${this.spec.label}（3D 可旋转）`)

    this.scene = new THREE.Scene()
    this.parts = buildPet3D(this.spec)
    this.scene.add(this.parts.root)
    // 影子独立挂场景：不随跳跃/蹦跳抬升，始终贴地。
    this.scene.add(this.parts.shadow)

    const camSpec = PET_CAMERA[this.spec.kind]
    this.camera = new THREE.PerspectiveCamera(camSpec.fov, width / height, 0.1, 100)
    this.camera.position.set(...camSpec.position)
    this.camera.lookAt(new THREE.Vector3(...camSpec.lookAt))

    const lighting = PET_LIGHTING[this.spec.kind]
    this.scene.add(new THREE.HemisphereLight(lighting.hemi.sky, lighting.hemi.ground, lighting.hemi.intensity))
    this.keyLight = new THREE.DirectionalLight(lighting.key.color, lighting.key.intensity)
    this.keyLight.position.set(...lighting.key.position)
    this.scene.add(this.keyLight)
    const fill = new THREE.DirectionalLight(lighting.fill.color, lighting.fill.intensity)
    fill.position.set(...lighting.fill.position)
    this.scene.add(fill)
    this.rimLight = new THREE.PointLight(opts.accent ?? this.spec.accent, lighting.rim.intensity, lighting.rim.distance)
    this.rimLight.position.set(...lighting.rim.position)
    this.scene.add(this.rimLight)
    const bottom = new THREE.PointLight(lighting.bottom.color, lighting.bottom.intensity, lighting.bottom.distance)
    bottom.position.set(...lighting.bottom.position)
    this.scene.add(bottom)
    if (lighting.front) {
      const front = new THREE.DirectionalLight(lighting.front.color, lighting.front.intensity)
      front.position.set(...lighting.front.position)
      this.scene.add(front)
    }

    this.container.innerHTML = ''
    this.container.appendChild(canvas)
    this.container.dataset.pet3d = 'ready'
    this.container.dataset.pet3dKind = this.spec.kind
    this.syncViewDataset()

    // 指针事件（拖拽环绕 + 视线跟随 + 点击部位反应）
    this.onPointerDown = (e) => this.handlePointerDown(e)
    this.onPointerMove = (e) => this.handlePointerMove(e)
    this.onPointerUp = (e) => this.handlePointerUp(e)
    this.onWheel = (e) => {
      e.preventDefault()
      this.zoomTarget = clamp(this.zoomTarget + Math.sign(e.deltaY) * -0.08, 0.75, 1.6)
    }
    this.onPointerLeave = () => {
      this.hoverTarget = 0
      this.pointer.inside = false
    }
    canvas.addEventListener('pointerdown', this.onPointerDown)
    canvas.addEventListener('pointermove', this.onPointerMove)
    canvas.addEventListener('pointerup', this.onPointerUp)
    canvas.addEventListener('pointerleave', this.onPointerLeave)
    canvas.addEventListener('wheel', this.onWheel, { passive: false })

    if (typeof ResizeObserver !== 'undefined') {
      this.resizeObserver = new ResizeObserver(() => this.resize())
      this.resizeObserver.observe(this.container)
    } else {
      this.onWindowResize = () => this.resize()
      window.addEventListener('resize', this.onWindowResize)
    }

    this.t0 = performance.now() / 1000
    this.lastMs = performance.now()
    this.tick()
  }

  /* ── 公开 API（与 PetSvg 同名） ───────────────────────────────────── */

  setMood(mood: PetMood3D): void {
    this.mood = mood
    this.zoomTarget = cameraZoomTarget(mood, this.hoverWeight)
  }

  triggerMoodAction(mood: PetMood3D): void {
    this.setMood(mood)
    if (mood === 'excited' || mood === 'happy') this.spin(0.9)
    if (mood === 'sleepy') this.zoomTarget = cameraZoomTarget('sleepy', this.hoverWeight)
  }

  setFacing(dir: 1 | -1): void {
    this.facingYaw = dir === 1 ? 0 : 0.24
  }

  jump(strength = 1): void {
    if (this.jumpY > 0.001 || this.disposed) return
    this.jumpV = 2.4 * Math.max(0.3, Math.min(1.6, strength))
  }

  spin(duration = 0.9): void {
    const now = performance.now()
    this.spinStart = now
    this.spinningUntil = now + duration * 1000
  }

  dance(duration = 3): void {
    this.dancingUntil = performance.now() + duration * 1000
  }

  setHover(weight: number): void {
    this.hoverTarget = clamp(weight, 0, 1)
    this.zoomTarget = cameraZoomTarget(this.mood, this.hoverTarget)
  }

  /**
   * 部位轻点反应（头=点头 / 背=侧倾 / 尾=甩尾）。
   * 与画布内的 pointerup 走同一条通道 —— 桌宠宿主把点击算在外层容器上
   * （画布 `pointer-events: none`，见 DesktopPet/PetOverlayView），所以必须公开。
   */
  poke(region: PetHitRegion): void {
    this.pokeRegion = region
    this.pokeAt = performance.now() / 1000 - this.t0
    this.hoverTarget = 1
  }

  /** 散步开关（桌宠「散步」行为）：只驱动步态相位，位移仍由宿主控制。 */
  setWalking(walking: boolean): void {
    this.walking = walking
  }

  setPointer(x: number, y: number, inside: boolean): void {
    // 与 PetSvg 同口径：入参是容器归一化后的 -1..1
    this.pointer = { x: clamp(x, -1, 1), y: clamp(y, -1, 1), inside }
  }

  lookAt(x: number, y: number): void {
    this.setPointer(x, y, true)
  }

  /** 预设视角（正面/45°/正侧/背面）；`free` 保持当前角度。 */
  setView(view: Pet3DView): void {
    this.view = view
    const azimuth = viewAzimuth(view)
    if (azimuth !== null) this.targetAzimuth = azimuth
    this.syncViewDataset()
  }

  setAutoRotate(on: boolean): void {
    this.autoRotate = on
    if (on) {
      this.view = 'free'
      this.targetAzimuth = null
    } else if (this.view === 'front') {
      this.targetAzimuth = 0
    }
    this.syncViewDataset()
  }

  orbitBy(dx: number, dy: number): void {
    this.azimuth -= dx * 0.008
    this.polar = clampPolar(this.polar - dy * 0.005)
    this.view = 'free'
    this.targetAzimuth = null
    this.syncViewDataset()
  }

  /** 读取当前视角状态（e2e/诊断用；不触发渲染）。 */
  viewState(): { view: Pet3DView; azimuth: number; polar: number } {
    return { view: this.view, azimuth: normalizeAzimuth(this.azimuth), polar: this.polar }
  }

  /* ── 指针 ───────────────────────────────────────────────────────── */

  private handlePointerDown(e: PointerEvent): void {
    this.dragging = true
    this.dragMoved = 0
    this.dragStart = { x: e.clientX, y: e.clientY, azimuth: this.azimuth, polar: this.polar }
    this.container.setPointerCapture?.(e.pointerId)
  }

  private handlePointerMove(e: PointerEvent): void {
    const rect = this.container.getBoundingClientRect()
    if (rect.width > 0 && rect.height > 0) {
      this.pointer = {
        x: ((e.clientX - rect.left) / rect.width) * 2 - 1,
        y: ((e.clientY - rect.top) / rect.height) * 2 - 1,
        inside: true,
      }
      this.hoverTarget = 1
    }
    if (!this.dragging) return
    const dx = e.clientX - this.dragStart.x
    const dy = e.clientY - this.dragStart.y
    this.dragMoved = Math.max(this.dragMoved, Math.hypot(dx, dy))
    if (this.dragMoved > 4) {
      this.azimuth = this.dragStart.azimuth - dx * 0.008
      this.polar = clampPolar(this.dragStart.polar - dy * 0.005)
      this.view = 'free'
      this.targetAzimuth = null
      this.syncViewDataset()
    }
  }

  private handlePointerUp(e: PointerEvent): void {
    const wasDrag = this.dragMoved > 6
    this.dragging = false
    this.container.releasePointerCapture?.(e.pointerId)
    if (wasDrag) return
    // 轻点：按纵向位置判定部位（上半 = 头，其余 = 背），给一次衰减冲量
    const rect = this.container.getBoundingClientRect()
    const ny = rect.height > 0 ? (e.clientY - rect.top) / rect.height : 0.5
    this.poke(ny < 0.45 ? 'head' : 'back')
  }

  /* ── 渲染循环 ───────────────────────────────────────────────────── */

  private tick = (): void => {
    if (this.disposed) return
    this.raf = requestAnimationFrame(this.tick)
    const nowMs = performance.now()
    const dt = Math.min(0.05, (nowMs - this.lastMs) / 1000)
    this.lastMs = nowMs
    const t = nowMs / 1000 - this.t0

    this.hoverWeight += (this.hoverTarget - this.hoverWeight) * (1 - Math.exp(-dt * 6))
    this.applyView(dt)
    this.applyPose(t, nowMs, dt)
    this.updateBursts(nowMs)
    this.renderer.render(this.scene, this.camera)
  }

  private applyView(dt: number): void {
    if (this.autoRotate) {
      this.azimuth += dt * 0.55
    } else if (this.targetAzimuth !== null) {
      this.azimuth = stepAngle(this.azimuth, this.targetAzimuth, dt)
    }
    this.zoom = stepCameraZoom(this.zoom, this.zoomTarget, dt)

    const camSpec = PET_CAMERA[this.spec.kind]
    // 取景中心按**实际模型高度**推导：spec 的 lookAt 是旧渲染器口径，兽形只有 0.3，
    // 直接沿用会让镜头对着肚子（人脸出画）。0.54×height ≈ 胸腔/面部之间。
    const target = new THREE.Vector3(0, this.parts.height * 0.54, 0)
    // 机位距离按「模型占画面 ~80% 高」反解，而不是照抄旧 spec 的固定距离：
    // 直接抄会让 380px 高的舞台里角色只占四成，看起来像缩略图。
    const fovRad = (camSpec.fov * Math.PI) / 180
    const fitRadius = this.parts.height / (2 * Math.tan(fovRad / 2) * 0.82)
    const baseRadius = Math.max(2.2, fitRadius)
    const radius = baseRadius / Math.max(0.4, this.zoom)
    const sinP = Math.sin(this.polar)
    this.camera.position.set(
      target.x + radius * sinP * Math.sin(this.azimuth),
      target.y + radius * Math.cos(this.polar),
      target.z + radius * sinP * Math.cos(this.azimuth),
    )
    this.camera.lookAt(target)
    // 主光跟随镜头方位角轻微偏转：正面受光、背面仍有轮廓，不出现纯黑面
    const lightAngle = this.azimuth * 0.6
    this.keyLight.position.set(Math.sin(lightAngle) * 2.4 + 0.8, 3.4, Math.cos(lightAngle) * 3.6 + 1.2)
    this.rimLight.position.set(-Math.sin(lightAngle) * 2.2, 2.4, -Math.cos(lightAngle) * 2.6)
    this.container.dataset.pet3dAzimuth = normalizeAzimuth(this.azimuth).toFixed(3)
    this.container.dataset.pet3dPolar = this.polar.toFixed(3)
    this.container.dataset.pet3dZoom = this.zoom.toFixed(3)
  }

  private applyPose(t: number, nowMs: number, dt: number): void {
    const blink = this.stepBlink(t)
    const twitch = this.stepTwitch(t)
    this.stepJump(dt)

    const dancing = this.dancingUntil > nowMs
    const spinning = this.spinningUntil > nowMs
    const pokeElapsed = this.pokeRegion ? t - this.pokeAt : null
    const gazeX = this.pointer.inside ? clamp(this.pointer.x, -1, 1) * 0.7 : 0
    const gazeY = this.pointer.inside ? clamp(this.pointer.y, -1, 1) * 0.7 : 0

    const input: PoseInput = {
      mood: this.mood,
      t,
      arousal: arousalOf(this.mood, this.hoverWeight),
      gazeX,
      gazeY,
      blinkScaleY: blink,
      twitchPhase: twitch,
      jumpY: this.jumpY,
      squash: this.squash,
      isWalking: this.walking,
      walkPhase: this.walking ? t * 2.2 : 0,
      isDancing: dancing,
      isSpinning: spinning,
      isBouncing: this.mood === 'happy' || this.mood === 'excited',
      isRubbingEyes: this.mood === 'sleepy',
      poke: this.pokeRegion ? { region: this.pokeRegion, elapsed: pokeElapsed ?? 0 } : null,
      focus: this.hoverWeight,
    }
    const pose = computePose(this.spec, input)
    const p = this.parts
    const H = this.parts.height

    // 根：跳跃/蹦跳/摇摆/侧倾 + 转身朝向
    p.root.position.set(pose.rootPosX * H * 0.23, pose.rootPosY * H * 0.30, 0)
    p.root.rotation.z = pose.rootRotZ
    p.root.rotation.y = this.facingYaw + (spinning ? this.spinAngle(nowMs) : 0)

    // 身体：呼吸/squash
    p.body.scale.set(pose.bodyScaleX, pose.bodyScaleY, pose.bodyScaleZ)
    p.body.position.y = pose.bodyPosY * H * 0.10

    // 头
    p.head.rotation.set(pose.headRotX, pose.headRotY, pose.headRotZ)

    // 耳（兽形尖耳；人形是精灵耳，同样有微动）
    const earBase = p.kind === 'beast' ? 0.20 : 0.06
    p.earL.rotation.z = earBase + pose.earRotZL
    p.earR.rotation.z = -earBase + pose.earRotZR

    // 眼/瞳
    p.eyeL.scale.y = pose.eyeScaleYL
    p.eyeR.scale.y = pose.eyeScaleYR
    p.eyeL.rotation.z = pose.eyeRotZL
    p.eyeR.rotation.z = pose.eyeRotZR
    p.pupilL.scale.setScalar(pose.pupilScale)
    p.pupilR.scale.setScalar(pose.pupilScale)
    p.pupilL.position.x = pose.pupilOffsetX * 3
    p.pupilR.position.x = pose.pupilOffsetX * 3
    p.pupilL.position.y = -pose.pupilOffsetY * 3
    p.pupilR.position.y = -pose.pupilOffsetY * 3

    // 眉
    if (p.browL && p.browR) {
      p.browL.rotation.z = 0.12 + pose.browRotZL
      p.browR.rotation.z = -0.12 + pose.browRotZR
    }

    // 嘴（切可见性，不重建几何）
    for (const [shape, obj] of Object.entries(p.mouth)) {
      obj.visible = pose.mouthShape === shape
    }
    if (pose.mouthShape === 'open') {
      p.mouth.open.scale.setScalar(1 + pose.mouthOpen * 0.5)
    }

    // 腮红
    for (const b of p.blush) {
      const mat = b.material as THREE.MeshBasicMaterial
      mat.opacity = pose.blushOpacity * 0.55
    }

    // 翼
    if (p.wingL && p.wingR) {
      p.wingL.rotation.y = -0.62 + pose.wingRotZL
      p.wingR.rotation.y = 0.62 + pose.wingRotZR
    }

    // 尾 / 后发摆动
    if (p.tail) {
      p.tail.rotation.y = pose.tailWag
      p.tail.rotation.x = pose.tailChainWave
    }
    if (p.hairBack) p.hairBack.rotation.y = pose.tailChainWave * 1.6

    // 光环
    if (p.halo) {
      const ring = p.halo.children[0] as THREE.Mesh | undefined
      if (ring) {
        const mat = ring.material as THREE.MeshToonMaterial
        mat.opacity = pose.haloOpacity * 0.85
        mat.transparent = true
      }
      p.halo.rotation.y = pose.haloRotZ
    }

    // 头顶饰物（位置以「基准 y」为锚，避免逐帧累加漂移）
    if (p.ornament) {
      const baseY = (p.ornament.userData.baseY as number | undefined) ?? p.ornament.position.y
      p.ornament.userData.baseY = baseY
      p.ornament.rotation.z = pose.flowerRotZ
      p.ornament.position.y = baseY + pose.flowerPosY * 0.4
    }

    // 手臂（打盹揉眼抬臂）
    if (p.armL && p.armR) {
      p.armL.rotation.z = -0.34 + pose.armLift * 0.85
      p.armR.rotation.z = 0.34 - pose.armLift * 0.85
    }

    // 影子随跳跃缩小变淡
    const jump = Math.max(0, this.jumpY)
    const s = 1 - Math.min(0.35, jump * 0.5)
    p.shadow.scale.setScalar(s)
    ;(p.shadow.material as THREE.MeshBasicMaterial).opacity = 0.16 * (1 - Math.min(0.6, jump * 0.8))
  }

  private spinAngle(nowMs: number): number {
    const total = Math.max(1, this.spinningUntil - this.spinStart)
    const progress = Math.min(1, (nowMs - this.spinStart) / total)
    // easeOutCubic：起步快、收尾稳，像是在原地转了个身
    const eased = 1 - (1 - progress) ** 3
    return eased * TWO_PI
  }

  private stepBlink(t: number): number {
    if (t >= this.blinkNextAt && this.blinkUntil === 0) {
      this.blinkUntil = t + 0.12
      const blink = this.spec.motion.blink
      this.blinkNextAt = t + blink.baseSeconds + this.rand() * blink.jitterSeconds
      if (!this.doubleBlink && this.rand() < blink.doubleBlinkProb) {
        this.doubleBlink = true
        this.blinkNextAt = Math.min(this.blinkNextAt, this.blinkUntil + 0.18)
      } else {
        this.doubleBlink = false
      }
    }
    if (this.blinkUntil > 0) {
      const remain = this.blinkUntil - t
      if (remain <= 0) { this.blinkUntil = 0; return 1 }
      // 半正弦：闭合再张开
      return Math.max(0.05, Math.abs(Math.cos((remain / 0.12) * Math.PI * 0.5)))
    }
    return 1
  }

  private stepTwitch(t: number): number {
    const every = this.spec.motion.earTwitch.everySeconds
    if (!Number.isFinite(every) || every <= 0 || every > 90) return 0
    if (t >= this.twitchNextAt && this.twitchUntil === 0) {
      this.twitchUntil = t + 0.35
      this.twitchNextAt = t + every * (0.6 + this.rand() * 0.8)
    }
    if (this.twitchUntil > 0) {
      if (t >= this.twitchUntil) { this.twitchUntil = 0; return 0 }
      return 1
    }
    return 0
  }

  private stepJump(dt: number): void {
    if (this.jumpV === 0 && this.jumpY <= 0) {
      this.squash += (1 - this.squash) * Math.min(1, dt * 12)
      return
    }
    this.jumpY += this.jumpV * dt
    this.jumpV -= 7.6 * dt
    if (this.jumpY <= 0) {
      this.jumpY = 0
      this.jumpV = 0
      this.squash = 0.86
    } else {
      this.squash = 1 + Math.min(0.08, this.jumpY * 0.18)
    }
  }

  /* ── 特效 ───────────────────────────────────────────────────────── */

  burst(color?: number, count = 24): void {
    const n = Math.max(6, Math.min(48, count))
    const positions = new Float32Array(n * 3)
    const velocities = new Float32Array(n * 3)
    for (let i = 0; i < n; i += 1) {
      const a = this.rand() * TWO_PI
      const r = 0.05 + this.rand() * 0.1
      positions[i * 3] = Math.cos(a) * r
      positions[i * 3 + 1] = this.parts.height * 0.62 + this.rand() * 0.12
      positions[i * 3 + 2] = Math.sin(a) * r + 0.1
      velocities[i * 3] = Math.cos(a) * (0.35 + this.rand() * 0.5)
      velocities[i * 3 + 1] = 0.7 + this.rand() * 0.9
      velocities[i * 3 + 2] = Math.sin(a) * (0.35 + this.rand() * 0.4)
    }
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    const mat = new THREE.PointsMaterial({
      color: color ?? this.spec.accent,
      size: 0.05,
      transparent: true,
      opacity: 1,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    })
    const points = new THREE.Points(geo, mat)
    points.name = 'pet3d-burst'
    this.scene.add(points)
    this.bursts.push({ points, velocities, bornAt: performance.now(), life: 1100 })
    // 上限保护：连续点击不会无界增长
    while (this.bursts.length > 4) {
      const old = this.bursts.shift()
      if (old) this.disposeBurst(old)
    }
  }

  private updateBursts(nowMs: number): void {
    for (let i = this.bursts.length - 1; i >= 0; i -= 1) {
      const b = this.bursts[i]
      const age = (nowMs - b.bornAt) / 1000
      if (age * 1000 >= b.life) {
        this.disposeBurst(b)
        this.bursts.splice(i, 1)
        continue
      }
      const pos = b.points.geometry.getAttribute('position') as THREE.BufferAttribute
      for (let k = 0; k < pos.count; k += 1) {
        const vy = b.velocities[k * 3 + 1] - 1.6 * age
        pos.setX(k, pos.getX(k) + b.velocities[k * 3] * 0.016)
        pos.setY(k, pos.getY(k) + vy * 0.016)
        pos.setZ(k, pos.getZ(k) + b.velocities[k * 3 + 2] * 0.016)
      }
      pos.needsUpdate = true
      ;(b.points.material as THREE.PointsMaterial).opacity = Math.max(0, 1 - (age * 1000) / b.life)
    }
  }

  private disposeBurst(b: Burst): void {
    this.scene.remove(b.points)
    b.points.geometry.dispose()
    ;(b.points.material as THREE.Material).dispose()
  }

  /* ── 生命周期 ───────────────────────────────────────────────────── */

  private resize(): void {
    if (this.disposed) return
    const w = Math.max(1, this.container.clientWidth)
    const h = Math.max(1, this.container.clientHeight)
    this.renderer.setSize(w, h, false)
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
  }

  private syncViewDataset(): void {
    this.container.dataset.pet3dView = this.view
    this.container.dataset.pet3d = 'ready'
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    cancelAnimationFrame(this.raf)
    const canvas = this.renderer.domElement
    canvas.removeEventListener('pointerdown', this.onPointerDown)
    canvas.removeEventListener('pointermove', this.onPointerMove)
    canvas.removeEventListener('pointerup', this.onPointerUp)
    canvas.removeEventListener('pointerleave', this.onPointerLeave)
    canvas.removeEventListener('wheel', this.onWheel)
    this.resizeObserver?.disconnect()
    if (this.onWindowResize) window.removeEventListener('resize', this.onWindowResize)
    for (const b of this.bursts) this.disposeBurst(b)
    this.bursts = []
    disposePet3D(this.parts)
    this.scene.remove(this.parts.root)
    this.renderer.dispose()
    canvas.parentElement?.removeChild(canvas)
    delete this.container.dataset.pet3d
    delete this.container.dataset.pet3dView
  }
}

/* ── 小工具 ──────────────────────────────────────────────────────── */

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v))
}

function hashString(s: string): number {
  let h = 2166136261
  for (let i = 0; i < s.length; i += 1) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return h >>> 0
}

function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/**
 * WebGL 可用性探测（组件在挂载前调用，决定 3D / 2.5D 降级）。
 *
 * 实现已抽到 `webglSupport.ts`（不依赖 three），这里只做兼容性再导出，
 * 让 PetView 能在 setup 阶段同步判定，避免探测晚到造成的布局位移。
 */
export { isWebGLAvailable } from './webglSupport'

