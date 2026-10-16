/**
 * DeviceTwinScene —— 冰箱数字孪生 3D 渲染器。
 *
 * 复用 island/Island3D.ts 的工程范式：
 * - 渲染器/相机/灯光/ResizeObserver/context-lost 同款写法；
 * - 质量分档（antialias / pixelRatio / shadow）按 quality 传入；
 * - 主题 accent 从 CSS 变量读入（readIslandThemeTokens 同套 parseCssColor）；
 * - 不做后处理 bloom，选中高亮用 emissive 染色（38 件，零管线风险）。
 *
 * 本类**只做渲染**：门角/爆炸进度/抽屉拉出量由外层 DeviceTwinStage.vue
 * 经 KinematicsController + ViewStateMachine 算好，每帧通过 setKinematicState()
 * 喂进来；本类不决策、不联锁。
 */

import * as THREE from 'three'
import { buildFridge, disposeFridge, pickableMeshes, type FridgeAssembly, type FridgePalette } from './fridgeBuilders'
import { FRIDGE_BOUNDS } from '../devices/fridge/FridgeModel'
import type { KinematicsState } from '../model/KinematicsController'
import type { DeviceViewState } from '../model/ViewStateMachine'

export type TwinQuality = 'high' | 'medium' | 'low'

export interface TwinSceneOptions {
  container: HTMLElement
  quality: TwinQuality
  palette?: FridgePalette
  onSelect?: (partId: string | null) => void
  onContextLost?: () => void
}

/** 每帧从外层喂进来的驱动状态（已经过缓动）。 */
export interface TwinFrameState {
  state: DeviceViewState
  /** 视图态缓动进度 0..1（product/open/explode/section/cycle） */
  progress: number
  kinematics: KinematicsState
}

/** 从 CSS 变量解析 hex（与 Island3D.readIslandThemeTokens 同口径）。 */
function parseCssColor(raw: string, fallback: number): number {
  const value = raw.trim()
  if (!value) return fallback
  if (value.startsWith('#')) {
    const hex = value.slice(1)
    if (hex.length === 3) return parseInt(hex.split('').map((c) => c + c).join(''), 16)
    if (hex.length >= 6) return parseInt(hex.slice(0, 6), 16)
    return fallback
  }
  const rgb = value.match(/rgba?\(([^)]+)\)/)
  if (rgb) {
    const [r, g, b] = rgb[1].split(/[,\s/]+/).filter(Boolean).map(Number)
    if ([r, g, b].every((n) => Number.isFinite(n))) {
      return ((Math.round(r) << 16) | (Math.round(g) << 8) | Math.round(b)) >>> 0
    }
  }
  return fallback
}

export function readAccentFromCss(fallback: number): number {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return fallback
  const style = getComputedStyle(document.documentElement)
  return parseCssColor(style.getPropertyValue('--accent'), fallback)
}

export class DeviceTwinScene {
  private readonly container: HTMLElement
  private readonly quality: TwinQuality
  private readonly renderer: THREE.WebGLRenderer
  private readonly scene: THREE.Scene
  private readonly camera: THREE.PerspectiveCamera
  private readonly assembly: FridgeAssembly
  private readonly hemi: THREE.HemisphereLight
  private readonly sun: THREE.DirectionalLight
  private readonly onSelect?: (id: string | null) => void
  private readonly onContextLost?: () => void

  private frame: TwinFrameState = {
    state: 'product',
    progress: 1,
    kinematics: {
      doors: { upperDoor: 0, lowerDoor: 0 },
      drawers: { upperDrawer: 0, lowerDrawer: 0 },
      locked: false,
    },
  }

  private selectedId: string | null = null
  private baseEmissive = new Map<string, number>()

  // 轨道相机（极简自研，不引 OrbitControls）
  private yaw = 0.55
  private pitch = 0.18
  private distance = 26
  private target = new THREE.Vector3(0, FRIDGE_BOUNDS.height / 2, 0)
  private yawGoal = this.yaw
  private pitchGoal = this.pitch
  private distanceGoal = this.distance

  private clock = new THREE.Clock()
  private rafId = 0
  private alive = true
  private paused = false

  // 指针交互
  private pointerMode: 'none' | 'orbit' = 'none'
  private pointerLast = { x: 0, y: 0 }
  private pointerStart = { x: 0, y: 0, time: 0 }
  private readonly raycaster = new THREE.Raycaster()
  private readonly pointerNdc = new THREE.Vector2()

  private resizeObserver?: ResizeObserver
  private intersectionObserver?: IntersectionObserver

  constructor(opts: TwinSceneOptions) {
    this.container = opts.container
    this.quality = opts.quality
    this.onSelect = opts.onSelect
    this.onContextLost = opts.onContextLost

    const width = Math.max(1, this.container.clientWidth)
    const height = Math.max(1, this.container.clientHeight)

    this.renderer = new THREE.WebGLRenderer({
      antialias: this.quality !== 'low',
      powerPreference: 'high-performance',
    })
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, this.quality === 'high' ? 2 : 1.25))
    this.renderer.setSize(width, height)
    this.renderer.shadowMap.enabled = this.quality === 'high'
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.domElement.style.cssText = 'width:100%;height:100%;display:block;touch-action:none;cursor:grab;'
    this.renderer.domElement.addEventListener('webglcontextlost', this.handleContextLost)
    this.container.appendChild(this.renderer.domElement)

    this.scene = new THREE.Scene()
    this.scene.background = new THREE.Color(0x1c1e22)

    this.camera = new THREE.PerspectiveCamera(42, width / height, 0.5, 100)
    this.updateCamera()

    // 灯光：半球环境光 + 主平行光（与 island 同思路，无后处理）
    this.hemi = new THREE.HemisphereLight(0xffffff, 0x404048, 1.1)
    this.scene.add(this.hemi)
    this.sun = new THREE.DirectionalLight(0xffffff, 1.6)
    this.sun.position.set(8, 20, 12)
    this.sun.castShadow = this.quality === 'high'
    if (this.sun.castShadow) {
      this.sun.shadow.mapSize.set(1024, 1024)
      this.sun.shadow.camera.left = -10
      this.sun.shadow.camera.right = 10
      this.sun.shadow.camera.top = 20
      this.sun.shadow.camera.bottom = -5
      this.sun.shadow.camera.near = 1
      this.sun.shadow.camera.far = 50
    }
    this.scene.add(this.sun)
    const rim = new THREE.DirectionalLight(0x88aaff, 0.4)
    rim.position.set(-8, 10, -10)
    this.scene.add(rim)

    this.assembly = buildFridge(opts.palette)
    this.scene.add(this.assembly.root)

    // 记录每个共享材质的初始 emissive（选中高亮后恢复）
    for (const mat of this.assembly.materials) {
      this.baseEmissive.set(mat.uuid, mat.emissive.getHex())
    }

    this.setupObservers()
    this.attachPointerHandlers()
    this.tick()
  }

  /** 外层每帧喂入最新运动状态（门角/爆炸进度/抽屉）。 */
  setFrameState(state: TwinFrameState): void {
    this.frame = state
  }

  /** 主题切换时刷新 accent（选中高亮色跟随 UI 主题）。 */
  setAccent(hex: number): void {
    if (this.selectedId) this.applySelection(this.selectedId, hex)
  }

  select(partId: string | null): void {
    this.selectedId = partId
    const accent = readAccentFromCss(0x60a5fa)
    // 先清掉旧高亮
    for (const [, mat] of this.assembly.parts) {
      mat.material.emissive.setHex(0x000000)
      mat.material.emissiveIntensity = 0
    }
    if (partId) this.applySelection(partId, accent)
  }

  dispose(): void {
    this.alive = false
    cancelAnimationFrame(this.rafId)
    this.resizeObserver?.disconnect()
    this.intersectionObserver?.disconnect()
    this.detachPointerHandlers()
    this.renderer.domElement.removeEventListener('webglcontextlost', this.handleContextLost)
    disposeFridge(this.assembly)
    this.renderer.dispose()
    if (this.renderer.domElement.parentNode) {
      this.renderer.domElement.parentNode.removeChild(this.renderer.domElement)
    }
  }

  /* ── 内部：每帧动画 ── */

  private applySelection(partId: string, accent: number): void {
    // 高亮整个部件（包括门组里所有子件——选中门时把把手/密封条一起点亮）
    const handle = this.assembly.parts.get(partId)
    if (!handle) return
    // 若选中门组部件，同组其它件也亮（视觉上整扇门被选中）
    const group = handle.group
    for (const [, h] of this.assembly.parts) {
      if (h.group === group && group !== 'fixed' && group !== 'compressor') {
        h.material.emissive.setHex(accent)
        h.material.emissiveIntensity = 0.35
      } else if (h.id === partId) {
        h.material.emissive.setHex(accent)
        h.material.emissiveIntensity = 0.35
      }
    }
  }

  private tick = (): void => {
    if (!this.alive) return
    this.rafId = requestAnimationFrame(this.tick)
    const dt = Math.min(this.clock.getDelta(), 0.05)
    if (this.paused) return

    // 缓动相机
    this.yaw += (this.yawGoal - this.yaw) * (1 - Math.exp(-dt * 6))
    this.pitch += (this.pitchGoal - this.pitch) * (1 - Math.exp(-dt * 6))
    this.distance += (this.distanceGoal - this.distance) * (1 - Math.exp(-dt * 6))
    this.updateCamera()

    this.applyKinematicTransforms()
    this.renderer.render(this.scene, this.camera)
  }

  private updateCamera(): void {
    const horiz = this.distance * Math.cos(this.pitch)
    this.camera.position.set(
      this.target.x + horiz * Math.sin(this.yaw),
      this.target.y + this.distance * Math.sin(this.pitch),
      this.target.z + horiz * Math.cos(this.yaw),
    )
    this.camera.lookAt(this.target)
  }

  /**
   * 把外层缓动后的运动状态应用到 three 场景图。
   * - 门：pivot.rotation.y = openAngle * doorOpen01（右铰链正开门外摆）
   * - 爆炸：anchor.position = base + explodeOffset * progress（explode 态）
   * - 抽屉：anchor.position.z = base.z + pullDistance * drawerPull
   */
  private applyKinematicTransforms(): void {
    const { state, progress, kinematics } = this.frame
    const explodeP = state === 'explode' ? progress : 0
    // 剖面态不做几何切割（留尾），但用 progress 压暗外壳示意
    for (const [, handle] of this.assembly.parts) {
      // 爆炸偏移（门组的枢轴位置也跟着爆炸前移）
      const target = handle.base.clone().addScaledVector(handle.explodeOffset, explodeP)
      // 抽屉拉出（explode 态下 KinematicsController 已锁定抽屉=0，不冲突）
      if (handle.pullDistance > 0) {
        target.z += handle.pullDistance * kinematics.drawers[handle.group as 'upperDrawer' | 'lowerDrawer']
      }
      handle.anchor.position.copy(target)
    }

    // 门开合（右铰链：绕 Y 正角让自由边向前 +z 摆开）
    const doorP = state === 'open' ? progress : 0
    // 若从 explode 态切回 open，门仍按 kinematics 实际开度收敛
    const upperAngle = handleOpenAngle(this.assembly, 'upperDoor')
    const lowerAngle = handleOpenAngle(this.assembly, 'lowerDoor')
    const upperPivot = this.assembly.doorPivots.get('upperDoor')
    const lowerPivot = this.assembly.doorPivots.get('lowerDoor')
    if (upperPivot) upperPivot.rotation.y = upperAngle * Math.max(doorP, kinematics.doors.upperDoor)
    if (lowerPivot) lowerPivot.rotation.y = lowerAngle * Math.max(doorP, kinematics.doors.lowerDoor)
  }

  /* ── 指针交互 ── */

  private attachPointerHandlers(): void {
    const el = this.renderer.domElement
    el.addEventListener('pointerdown', this.handlePointerDown)
    el.addEventListener('pointermove', this.handlePointerMove)
    el.addEventListener('pointerup', this.handlePointerUp)
    el.addEventListener('pointercancel', this.handlePointerUp)
    el.addEventListener('wheel', this.handleWheel, { passive: false })
  }

  private detachPointerHandlers(): void {
    const el = this.renderer.domElement
    el.removeEventListener('pointerdown', this.handlePointerDown)
    el.removeEventListener('pointermove', this.handlePointerMove)
    el.removeEventListener('pointerup', this.handlePointerUp)
    el.removeEventListener('pointercancel', this.handlePointerUp)
    el.removeEventListener('wheel', this.handleWheel)
  }

  private handlePointerDown = (event: PointerEvent): void => {
    this.renderer.domElement.setPointerCapture?.(event.pointerId)
    this.pointerStart = { x: event.clientX, y: event.clientY, time: performance.now() }
    this.pointerLast = { x: event.clientX, y: event.clientY }
    this.pointerMode = 'orbit'
    this.renderer.domElement.style.cursor = 'grabbing'
  }

  private handlePointerMove = (event: PointerEvent): void => {
    if (this.pointerMode !== 'orbit') return
    const dx = event.clientX - this.pointerLast.x
    const dy = event.clientY - this.pointerLast.y
    this.pointerLast = { x: event.clientX, y: event.clientY }
    this.yawGoal += dx * 0.008
    this.pitchGoal = THREE.MathUtils.clamp(this.pitchGoal + dy * 0.006, -0.2, 1.1)
  }

  private handlePointerUp = (event: PointerEvent): void => {
    const movement = Math.hypot(event.clientX - this.pointerStart.x, event.clientY - this.pointerStart.y)
    const elapsed = performance.now() - this.pointerStart.time
    const wasClick = movement < 6 && elapsed < 420 && event.button === 0
    this.pointerMode = 'none'
    this.renderer.domElement.releasePointerCapture?.(event.pointerId)
    this.renderer.domElement.style.cursor = 'grab'
    if (wasClick) {
      const id = this.pickAt(event)
      this.onSelect?.(id)
    }
  }

  private handleWheel = (event: WheelEvent): void => {
    event.preventDefault()
    this.distanceGoal = THREE.MathUtils.clamp(this.distanceGoal + Math.sign(event.deltaY) * 2, 14, 40)
  }

  private pickAt(event: MouseEvent): string | null {
    const rect = this.renderer.domElement.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return null
    this.pointerNdc.set(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1,
    )
    this.raycaster.setFromCamera(this.pointerNdc, this.camera)
    const hits = this.raycaster.intersectObjects(pickableMeshes(this.assembly), false)
    if (hits.length === 0) return null
    const partId = hits[0].object.userData.partId
    return typeof partId === 'string' ? partId : null
  }

  private handleContextLost = (event: Event): void => {
    event.preventDefault()
    this.paused = true
    this.onContextLost?.()
  }

  private setupObservers(): void {
    this.resizeObserver = new ResizeObserver(() => {
      const width = Math.max(1, this.container.clientWidth)
      const height = Math.max(1, this.container.clientHeight)
      this.camera.aspect = width / height
      this.camera.updateProjectionMatrix()
      this.renderer.setSize(width, height)
    })
    this.resizeObserver.observe(this.container)

    if (typeof IntersectionObserver === 'function') {
      this.intersectionObserver = new IntersectionObserver(
        (entries) => { for (const e of entries) this.paused = !e.isIntersecting },
        { threshold: 0.02 },
      )
      this.intersectionObserver.observe(this.container)
    }
  }
}

/** 从装配里取某个门组的最大开合角（弧度）。 */
function handleOpenAngle(assembly: FridgeAssembly, group: 'upperDoor' | 'lowerDoor'): number {
  for (const [, h] of assembly.parts) {
    if (h.group === group && h.openAngle > 0) return h.openAngle
  }
  return group === 'upperDoor' ? 1.83 : 1.57
}
