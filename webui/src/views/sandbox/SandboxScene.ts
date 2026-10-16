/**
 * SandboxScene —— 微缩高铁环线沙盘渲染器（任务书 B）。
 *
 * 职责：
 * - 木桌 + 蓝图 + 闭环双线（2 站 + 一段桥）+ 3 列压缩编组，全部程序化几何/Canvas 纹理。
 * - **运行图驱动**：列车位置由 schedule.distanceAt / trainStateAt 纯函数给出；
 *   setTime(t) 是唯一状态输入 —— 时间轴拖回同一 t，列车位置/朝向/车轮转角完全一致。
 * - 昼夜切换：夜晚站灯 emissive + 少量暖 PointLight 真的照到地面。
 * - 可选小雨：只加雨丝 + 微湿反射，**不压暗画面**（不加雾/蒙层）。
 * - 点选列车 → onSelect(id)；选中用地面光环高亮，不改车体色。
 *
 * 诚实：构造失败（无 WebGL）直接抛错，由视图降级 2D 示意图，不假装 3D。
 * 本文件是 .ts（非 .vue/.css），不经过样式守卫；配色为 three 数字常量。
 */
import * as THREE from 'three'
import {
  LOOP_LENGTH,
  TRAIN_COUNT,
  trainStateAt,
  type StationId,
} from './schedule'
import { makeBlueprintTexture, makeConcreteTexture, makeWoodTexture } from './textures'
import { makeSaltedRng, SANDBOX_SEED } from './rng'
import {
  exceedsDragThreshold,
  trainIndexFromEntityId,
  type PointerPos,
} from './pickLogic'

export type SandboxEntityKind = 'train'

export interface SandboxPick {
  kind: SandboxEntityKind
  index: number
}

export interface SandboxSceneOptions {
  container: HTMLElement
  onSelect?: (pick: SandboxPick | null) => void
  onContextLost?: () => void
}

interface TrainRig {
  group: THREE.Group
  body: THREE.Mesh
  roof: THREE.Mesh
  wheels: THREE.Mesh[]
  windowMat: THREE.MeshStandardMaterial
}

const BRIDGE_U0 = 0.30
const BRIDGE_U1 = 0.44
const BRIDGE_H = 2.6

function loopPoint(u: number, curve: THREE.CatmullRomCurve3): THREE.Vector3 {
  const pt = curve.getPointAt(((u % 1) + 1) % 1)
  // 桥区段抬升（越江），两端缓坡
  let y = 0
  if (u > BRIDGE_U0 && u < BRIDGE_U1) {
    const mid = (BRIDGE_U0 + BRIDGE_U1) / 2
    const half = (BRIDGE_U1 - BRIDGE_U0) / 2
    const x = (u - mid) / half // -1..1
    y = BRIDGE_H * (1 - x * x) * 0.5 + BRIDGE_H * 0.5 // 弧形坡
  }
  pt.y = y
  return pt
}

export class SandboxScene {
  private readonly container: HTMLElement
  private readonly onSelect?: (p: SandboxPick | null) => void
  private readonly onContextLost?: () => void

  private renderer: THREE.WebGLRenderer
  private scene: THREE.Scene
  private camera: THREE.PerspectiveCamera
  private curve: THREE.CatmullRomCurve3
  private trains: TrainRig[] = []
  private selectedRing: THREE.Mesh
  private lampLights: THREE.PointLight[] = []
  private windowMats: THREE.MeshStandardMaterial[] = []
  private stationSignMats: THREE.MeshBasicMaterial[] = []
  private rain: THREE.Points | null = null
  private rainMat: THREE.PointsMaterial | null = null

  private readonly raycaster = new THREE.Raycaster()
  private readonly ndc = new THREE.Vector2()
  private clock = new THREE.Clock()
  private rafId = 0
  private alive = false

  // 简易轨道相机
  private camYaw = -Math.PI / 4
  private camPitch = 0.62
  private camDist = 34
  private readonly camTarget = new THREE.Vector3(0, 1, 0)
  private dragging: 'none' | 'orbit' | 'pan' = 'none'
  private lastPointer = { x: 0, y: 0 }
  // 点选 vs 拖拽：记录按下位置；位移越过阈值才开始转镜头，否则松开算一次点击
  private pointerDownPos: PointerPos = { x: 0, y: 0 }
  private dragActivated = false

  private resizeObserver?: ResizeObserver

  constructor(opts: SandboxSceneOptions) {
    this.container = opts.container
    this.onSelect = opts.onSelect
    this.onContextLost = opts.onContextLost

    const width = Math.max(1, this.container.clientWidth)
    const height = Math.max(1, this.container.clientHeight)

    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance' })
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2))
    this.renderer.setSize(width, height)
    this.renderer.shadowMap.enabled = true
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap
    this.renderer.domElement.style.cssText =
      'width:100%;height:100%;display:block;touch-action:none;cursor:grab;'
    this.renderer.domElement.addEventListener('webglcontextlost', this.handleContextLost)
    this.container.appendChild(this.renderer.domElement)

    this.scene = new THREE.Scene()
    this.scene.background = new THREE.Color(0x2b2f36)
    this.scene.fog = new THREE.Fog(0x2b2f36, 46, 90)

    this.camera = new THREE.PerspectiveCamera(46, width / height, 0.1, 200)

    this.curve = this.buildTrackPath()
    this.buildEnvironment()
    this.buildTrack()
    this.buildStations()
    this.trains = this.buildTrains()

    // 选中光环（不改变车体色）
    const ringGeo = new THREE.RingGeometry(1.4, 1.9, 40)
    const ringMat = new THREE.MeshBasicMaterial({
      color: 0xffcf6e,
      transparent: true,
      opacity: 0.9,
      side: THREE.DoubleSide,
      depthWrite: false,
    })
    this.selectedRing = new THREE.Mesh(ringGeo, ringMat)
    this.selectedRing.rotation.x = -Math.PI / 2
    this.selectedRing.visible = false
    this.scene.add(this.selectedRing)

    this.bindPointer()
    this.resizeObserver = new ResizeObserver(() => this.handleResize())
    this.resizeObserver.observe(this.container)

    this.alive = true
    this.clock.start()
    this.tick()
    // [临时诊断] 暴露实例供控制台调试
    ;(window as unknown as { __sbScene?: SandboxScene }).__sbScene = this
  }

  /* ── 几何搭建 ── */

  private buildTrackPath(): THREE.CatmullRomCurve3 {
    // 闭环圆角矩形（XZ 平面），起点 = 站 A。右侧直线 = 越江桥。
    const pts: THREE.Vector3[] = [
      new THREE.Vector3(-11, 0, -6),
      new THREE.Vector3(0, 0, -6.4),
      new THREE.Vector3(11, 0, -6),
      new THREE.Vector3(13.2, 0, -2),
      new THREE.Vector3(13.4, 0, 4), // 桥
      new THREE.Vector3(11, 0, 6),
      new THREE.Vector3(0, 0, 6.4),
      new THREE.Vector3(-11, 0, 6),
      new THREE.Vector3(-13.2, 0, 2),
      new THREE.Vector3(-13.2, 0, -2),
    ]
    const curve = new THREE.CatmullRomCurve3(pts, true, 'catmullrom', 0.12)
    curve.arcLengthDivisions = 400
    return curve
  }

  private buildEnvironment(): void {
    // 木桌
    const wood = makeWoodTexture()
    wood.repeat.set(3, 2)
    const tableTop = new THREE.Mesh(
      new THREE.BoxGeometry(44, 0.8, 32),
      new THREE.MeshStandardMaterial({ map: wood, roughness: 0.85, metalness: 0 }),
    )
    tableTop.position.y = -0.4
    tableTop.receiveShadow = true
    this.scene.add(tableTop)
    // 桌腿
    const legMat = new THREE.MeshStandardMaterial({ color: 0x5c3a1e, roughness: 0.9 })
    for (const [x, z] of [[-20, -14], [20, -14], [-20, 14], [20, 14]] as const) {
      const leg = new THREE.Mesh(new THREE.BoxGeometry(1.2, 6, 1.2), legMat)
      leg.position.set(x, -3.8, z)
      this.scene.add(leg)
    }

    // 蓝图（散落 2 张，平铺桌面）
    const bp1 = new THREE.Mesh(
      new THREE.PlaneGeometry(9, 6.7),
      new THREE.MeshStandardMaterial({ map: makeBlueprintTexture(21), roughness: 0.95 }),
    )
    bp1.rotation.x = -Math.PI / 2
    bp1.rotation.z = 0.12
    bp1.position.set(-16, 0.02, 10)
    this.scene.add(bp1)
    const bp2 = new THREE.Mesh(
      new THREE.PlaneGeometry(7, 5.2),
      new THREE.MeshStandardMaterial({ map: makeBlueprintTexture(33), roughness: 0.95 }),
    )
    bp2.rotation.x = -Math.PI / 2
    bp2.rotation.z = -0.2
    bp2.position.set(16, 0.02, -11)
    this.scene.add(bp2)

    // 混凝土底座（沙盘底板）
    const concrete = makeConcreteTexture()
    concrete.repeat.set(6, 4)
    const base = new THREE.Mesh(
      new THREE.BoxGeometry(34, 0.4, 24),
      new THREE.MeshStandardMaterial({ map: concrete, roughness: 0.9 }),
    )
    base.position.y = 0.2
    base.receiveShadow = true
    this.scene.add(base)

    // 灯光
    const hemi = new THREE.HemisphereLight(0xfff2dd, 0x6b5a48, 1.0)
    this.scene.add(hemi)
    const sun = new THREE.DirectionalLight(0xffe6c0, 2.2)
    sun.position.set(18, 26, 12)
    sun.castShadow = true
    sun.shadow.mapSize.set(1024, 1024)
    sun.shadow.camera.left = -24
    sun.shadow.camera.right = 24
    sun.shadow.camera.top = 24
    sun.shadow.camera.bottom = -24
    this.scene.add(sun)
    ;(this as unknown as { hemiLight: THREE.HemisphereLight }).hemiLight = hemi
    ;(this as unknown as { sunLight: THREE.DirectionalLight }).sunLight = sun
  }

  private buildTrack(): void {
    // 双线：沿闭环生成铁轨（TubeGeometry 沿两条偏移曲线）
    const railMat = new THREE.MeshStandardMaterial({ color: 0x8a8f96, roughness: 0.4, metalness: 0.7 })
    for (const offset of [-0.35, 0.35]) {
      const pts: THREE.Vector3[] = []
      for (let i = 0; i <= 120; i++) {
        const u = i / 120
        const p = loopPoint(u, this.curve)
        const tan = this.curve.getTangentAt(u)
        const normal = new THREE.Vector3(-tan.z, 0, tan.x).normalize()
        p.addScaledVector(normal, offset)
        p.y += 0.15
        pts.push(p)
      }
      const railCurve = new THREE.CatmullRomCurve3(pts, true)
      const tube = new THREE.Mesh(new THREE.TubeGeometry(railCurve, 160, 0.045, 6, true), railMat)
      tube.castShadow = true
      this.scene.add(tube)
    }
    // 道砟床（扁平盒沿中线）
    const bedPts: THREE.Vector3[] = []
    for (let i = 0; i <= 120; i++) bedPts.push(loopPoint(i / 120, this.curve))
    const bedCurve = new THREE.CatmullRomCurve3(bedPts, true)
    const bed = new THREE.Mesh(
      new THREE.TubeGeometry(bedCurve, 160, 0.55, 6, true),
      new THREE.MeshStandardMaterial({ color: 0x55504a, roughness: 1 }),
    )
    bed.position.y = 0.05
    bed.receiveShadow = true
    this.scene.add(bed)
  }

  private buildStations(): void {
    const stationU: Record<StationId, number> = { A: 0, B: 0.5 }
    const signColors: number[] = [0xffcf6e, 0x9fd6c0]
    let i = 0
    for (const id of ['A', 'B'] as const) {
      const u = stationU[id]
      const p = loopPoint(u, this.curve)
      const tan = this.curve.getTangentAt(u)
      const normal = new THREE.Vector3(-tan.z, 0, tan.x).normalize()
      // 站台
      const platform = new THREE.Mesh(
        new THREE.BoxGeometry(4.5, 0.35, 1.6),
        new THREE.MeshStandardMaterial({ color: 0xb8b0a4, roughness: 0.9 }),
      )
      platform.position.copy(p).addScaledVector(normal, 1.4)
      platform.position.y = 0.4
      platform.castShadow = true
      platform.receiveShadow = true
      this.scene.add(platform)
      // 站名牌（emissive，夜晚自发光）
      const signMat = new THREE.MeshBasicMaterial({ color: signColors[i % 2] })
      const sign = new THREE.Mesh(new THREE.BoxGeometry(1.6, 0.5, 0.12), signMat)
      sign.position.copy(p).addScaledVector(normal, 1.4)
      sign.position.y = 1.6
      this.scene.add(sign)
      this.stationSignMats.push(signMat)
      // 暖光点（夜晚真的照地面）
      const lamp = new THREE.PointLight(0xffb066, 0, 9, 2)
      lamp.position.copy(p).addScaledVector(normal, 1.4)
      lamp.position.y = 2.2
      this.scene.add(lamp)
      this.lampLights.push(lamp)
      i++
    }
  }

  private buildTrains(): TrainRig[] {
    const bodyColors = [0xb3402f, 0x2f7f7a, 0xc79a3a]
    const rng = makeSaltedRng(SANDBOX_SEED, 'trainWindows')
    const rigs: TrainRig[] = []
    for (let i = 0; i < TRAIN_COUNT; i++) {
      const g = new THREE.Group()
      const bodyMat = new THREE.MeshStandardMaterial({ color: bodyColors[i], roughness: 0.5, metalness: 0.1 })
      const body = new THREE.Mesh(new THREE.BoxGeometry(2.4, 0.7, 0.8), bodyMat)
      body.position.y = 0.7
      body.castShadow = true
      g.add(body)
      const roofMat = new THREE.MeshStandardMaterial({ color: 0xd8d4cc, roughness: 0.6 })
      const roof = new THREE.Mesh(new THREE.BoxGeometry(2.0, 0.18, 0.7), roofMat)
      roof.position.y = 1.13
      g.add(roof)
      // 车窗（emissive，夜晚亮）
      const windowMat = new THREE.MeshStandardMaterial({
        color: 0x223038,
        emissive: 0xffd9a0,
        emissiveIntensity: 0,
      })
      const win = new THREE.Mesh(new THREE.BoxGeometry(2.2, 0.28, 0.82), windowMat)
      win.position.y = 0.85
      g.add(win)
      this.windowMats.push(windowMat)
      // 车轮
      const wheelMat = new THREE.MeshStandardMaterial({ color: 0x222222, roughness: 0.7 })
      const wheels: THREE.Mesh[] = []
      for (const wx of [-0.8, 0.8]) {
        const wheel = new THREE.Mesh(new THREE.CylinderGeometry(0.22, 0.22, 0.12, 12), wheelMat)
        wheel.rotation.z = Math.PI / 2
        wheel.position.set(wx, 0.28, 0)
        g.add(wheel)
        wheels.push(wheel)
      }
      g.userData.entityId = `train:${i}`
      g.userData.trainIndex = i
      void rng
      this.scene.add(g)
      rigs.push({ group: g, body, roof, wheels, windowMat })
    }
    return rigs
  }

  /* ── 时间驱动（State From Time） ── */

  /** 设置全局沙盘时间（秒）。所有列车位置/朝向/车轮由 schedule 纯函数推导。 */
  setTime(t: number): void {
    for (let i = 0; i < this.trains.length; i++) {
      const state = trainStateAt(i, t)
      const u = state.distance / LOOP_LENGTH
      const p = loopPoint(u, this.curve)
      const tan = this.curve.getTangentAt(u)
      const rig = this.trains[i]
      rig.group.position.set(p.x, p.y + 0.15, p.z)
      rig.group.rotation.y = Math.atan2(tan.x, tan.z)
      // 车轮随行进距离滚动，车体不翻滚
      const wheelAngle = state.wheelDistance / 0.22
      for (const w of rig.wheels) w.rotation.x = wheelAngle
    }
  }

  setDayFactor(f: number): void {
    const hemi = (this as unknown as { hemiLight: THREE.HemisphereLight }).hemiLight
    const sun = (this as unknown as { sunLight: THREE.DirectionalLight }).sunLight
    // 白天：暖白光足；夜晚：压低环境光，只留站灯
    hemi.intensity = 0.25 + 0.75 * f
    sun.intensity = 0.15 + 2.1 * f
    sun.color.setHSL(0.09, 0.5, 0.5 + 0.3 * f)
    ;(this.scene.fog as THREE.Fog).color.setHex(f > 0.5 ? 0x2b2f36 : 0x1a1c22)
    this.scene.background = new THREE.Color(f > 0.5 ? 0x3a4250 : 0x14161c)
    // 车窗/站灯夜晚亮起（emissive 为主，真落地）
    for (const m of this.windowMats) m.emissiveIntensity = (1 - f) * 1.4
    for (const s of this.stationSignMats) {
      ;(s as THREE.MeshBasicMaterial).color.setHex(f > 0.5 ? 0xffcf6e : 0xffe6b0)
    }
    for (const l of this.lampLights) l.intensity = (1 - f) * 1.6
  }

  setRain(on: boolean): void {
    if (on && !this.rain) this.buildRain()
    if (this.rain) this.rain.visible = on
  }

  private buildRain(): void {
    // 雨丝：Points 下落，不加雾/蒙层 → 不压暗画面
    const count = 350
    const pos = new Float32Array(count * 3)
    const rng = makeSaltedRng(SANDBOX_SEED, 'rain')
    for (let i = 0; i < count; i++) {
      pos[i * 3] = (rng() - 0.5) * 40
      pos[i * 3 + 1] = rng() * 18
      pos[i * 3 + 2] = (rng() - 0.5) * 28
    }
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pos, 3))
    this.rainMat = new THREE.PointsMaterial({
      color: 0xbcd0e0,
      size: 0.08,
      transparent: true,
      opacity: 0.5,
      depthWrite: false,
    })
    this.rain = new THREE.Points(geo, this.rainMat)
    this.scene.add(this.rain)
  }

  select(index: number | null): void {
    if (index === null || index === undefined) {
      this.selectedRing.visible = false
      return
    }
    const rig = this.trains[index]
    if (!rig) return
    this.selectedRing.visible = true
    this.selectedRing.position.set(rig.group.position.x, 0.25, rig.group.position.z)
  }

  /* ── 交互 ── */

  private bindPointer(): void {
    const el = this.renderer.domElement
    el.addEventListener('pointerdown', this.onPointerDown)
    window.addEventListener('pointermove', this.onPointerMove)
    window.addEventListener('pointerup', this.onPointerUp)
    el.addEventListener('wheel', this.onWheel, { passive: false })
  }

  private onPointerDown = (e: PointerEvent): void => {
    this.pointerDownPos = { x: e.clientX, y: e.clientY }
    this.lastPointer = { x: e.clientX, y: e.clientY }
    this.dragActivated = false
    this.dragging = e.button === 2 || e.shiftKey ? 'pan' : 'orbit'
    // [临时诊断]
    console.log('[sb-pick] pointerdown', { button: e.button, x: e.clientX, y: e.clientY, dragging: this.dragging })
  }

  private onPointerMove = (e: PointerEvent): void => {
    if (this.dragging === 'none') return
    // 拖拽阈值未越过前不转镜头：手指 1px 的自然抖动不该把列车从光标下推开。
    if (!this.dragActivated) {
      if (exceedsDragThreshold(this.pointerDownPos, { x: e.clientX, y: e.clientY })) {
        this.dragActivated = true
        this.lastPointer = { x: e.clientX, y: e.clientY }
      } else {
        return
      }
    }
    const dx = e.clientX - this.lastPointer.x
    const dy = e.clientY - this.lastPointer.y
    this.lastPointer = { x: e.clientX, y: e.clientY }
    if (this.dragging === 'orbit') {
      this.camYaw -= dx * 0.006
      this.camPitch = Math.max(0.15, Math.min(1.35, this.camPitch - dy * 0.005))
    } else {
      const pan = new THREE.Vector3()
      const right = new THREE.Vector3(Math.cos(this.camYaw), 0, -Math.sin(this.camYaw))
      pan.addScaledVector(right, -dx * 0.03)
      pan.z += dy * 0.03
      this.camTarget.add(pan)
    }
  }

  private onPointerUp = (e: PointerEvent): void => {
    const wasDrag = this.dragging
    // 只有「按下期间没怎么动」才算一次点选；拖拽留给轨道相机，不做 raycast。
    const wasClick = wasDrag === 'orbit' && !e.shiftKey && !this.dragActivated
    this.dragging = 'none'
    this.dragActivated = false
    // [临时诊断]
    console.log('[sb-pick] pointerup', { wasDrag, wasClick, x: e.clientX, y: e.clientY })
    if (wasClick) this.pickAt(e)
  }

  private onWheel = (e: WheelEvent): void => {
    e.preventDefault()
    this.camDist = Math.max(12, Math.min(70, this.camDist + e.deltaY * 0.02))
  }

  private pickAt(e: MouseEvent): void {
    const rect = this.renderer.domElement.getBoundingClientRect()
    if (rect.width === 0) return
    this.ndc.set(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1,
    )
    // 列车在 setTime 里改了 position/rotation；three 的 Raycaster 不会自己更新世界矩阵，
    // 不刷新就会对「上一帧的旧矩阵」做命中，点不到正在动的列车。
    this.scene.updateMatrixWorld()
    // 投影前强制把相机宽高比同步到实际 CSS 视口（与渲染共用同一尺寸）。
    const cssAspect = rect.width / rect.height
    if (Math.abs(this.camera.aspect - cssAspect) > 0.001) {
      console.log('[sb-pick] aspect mismatch: cam=' + this.camera.aspect.toFixed(3) + ' css=' + cssAspect.toFixed(3) + ' -> sync')
      this.camera.aspect = cssAspect
      this.camera.updateProjectionMatrix()
    }
    this.camera.updateMatrixWorld()
    this.raycaster.setFromCamera(this.ndc, this.camera)

    // ── 主路径：raycast 整列车 ──
    const rigGroups = this.trains.map((t) => t.group)
    const hits = this.raycaster.intersectObjects(rigGroups, true)

    // ── 兜底：屏幕空间邻近度（列车在屏上仅 ~60x20px，raycast 点偏几像素就空）──
    const clickX = e.clientX - rect.left
    const clickY = e.clientY - rect.top
    const proj = new THREE.Vector3()
    const worldPos = new THREE.Vector3()
    let bestIdx: number | null = null
    let bestDist = Infinity
    const PROXIMITY_RADIUS_PX = 30
    for (let i = 0; i < this.trains.length; i++) {
      const rig = this.trains[i]
      rig.group.getWorldPosition(worldPos)
      proj.set(worldPos.x, worldPos.y + 0.85, worldPos.z)
      proj.project(this.camera)
      if (proj.z > 1 || proj.z < -1) continue // 在相机背后
      const sx = ((proj.x + 1) / 2) * rect.width
      const sy = ((-proj.y + 1) / 2) * rect.height
      const d = Math.hypot(sx - clickX, sy - clickY)
      console.log(`[sb-pick] train ${i} screen=(${sx.toFixed(0)},${sy.toFixed(0)}) click=(${clickX.toFixed(0)},${clickY.toFixed(0)}) d=${d.toFixed(1)}px`)
      if (d < bestDist) {
        bestDist = d
        bestIdx = i
      }
    }

    // [临时诊断]
    const rendererSize = this.renderer.getSize(new THREE.Vector2())
    console.log('[sb-pick] pickAt', {
      rect: { left: Math.round(rect.left), top: Math.round(rect.top), w: Math.round(rect.width), h: Math.round(rect.height) },
      ndc: [Number(this.ndc.x.toFixed(3)), Number(this.ndc.y.toFixed(3))],
      camAspect: Number(this.camera.aspect.toFixed(3)),
      rendererSize: rendererSize.toArray().map((v) => Math.round(v)),
      camPos: this.camera.position.toArray().map((v) => Number(v.toFixed(2))),
      hits: hits.length,
      hitObj: hits[0]?.object?.type ?? null,
      proximityBest: bestIdx,
      proximityDist: Number(bestDist.toFixed(1)),
      radius: PROXIMITY_RADIUS_PX,
    })

    // 优先 raycast 精确命中
    if (hits.length > 0) {
      let obj: THREE.Object3D | null = hits[0].object
      while (obj && obj.userData.entityId === undefined) obj = obj.parent
      const index = trainIndexFromEntityId(obj?.userData.entityId)
      console.log('[sb-pick] walk-up', { objType: obj?.type, entityId: obj?.userData.entityId, index })
      if (index !== null) {
        this.onSelect?.({ kind: 'train', index })
        return
      }
    }

    // 兜底：邻近度内选最近列车
    if (bestIdx !== null && bestDist <= PROXIMITY_RADIUS_PX) {
      console.log(`[sb-pick] proximity fallback → train ${bestIdx}`)
      this.onSelect?.({ kind: 'train', index: bestIdx })
      return
    }

    // 空白处点击 → 取消选中
    this.onSelect?.(null)
  }

  private handleResize(): void {
    const w = Math.max(1, this.container.clientWidth)
    const h = Math.max(1, this.container.clientHeight)
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
    this.renderer.setSize(w, h)
  }

  private handleContextLost = (e: Event): void => {
    e.preventDefault()
    this.alive = false
    this.onContextLost?.()
  }

  private tick = (): void => {
    if (!this.alive) return
    this.rafId = requestAnimationFrame(this.tick)
    const dt = Math.min(this.clock.getDelta(), 0.05)

    // 相机始终更新（暂停世界时相机仍可动）
    const cx = this.camTarget.x + this.camDist * Math.cos(this.camPitch) * Math.sin(this.camYaw)
    const cy = this.camTarget.y + this.camDist * Math.sin(this.camPitch)
    const cz = this.camTarget.z + this.camDist * Math.cos(this.camPitch) * Math.cos(this.camYaw)
    this.camera.position.set(cx, cy, cz)
    this.camera.lookAt(this.camTarget)

    // 雨下落（独立于暂停？小雨是天气，继续；但 State From Time 要求 deterministic——
    // 雨用固定种子位置，按 dt 下移后模高，仅视觉层，不影响列车状态）
    if (this.rain && this.rain.visible && this.rainMat) {
      const attr = this.rain.geometry.attributes.position as THREE.BufferAttribute
      for (let i = 0; i < attr.count; i++) {
        let y = attr.getY(i) - dt * 12
        if (y < 0) y = 18
        attr.setY(i, y)
      }
      attr.needsUpdate = true
    }

    this.renderer.render(this.scene, this.camera)
  }

  dispose(): void {
    this.alive = false
    cancelAnimationFrame(this.rafId)
    this.resizeObserver?.disconnect()
    const el = this.renderer.domElement
    el.removeEventListener('pointerdown', this.onPointerDown)
    window.removeEventListener('pointermove', this.onPointerMove)
    window.removeEventListener('pointerup', this.onPointerUp)
    el.removeEventListener('wheel', this.onWheel)
    el.removeEventListener('webglcontextlost', this.handleContextLost)
    this.renderer.dispose()
    el.remove()
  }
}
