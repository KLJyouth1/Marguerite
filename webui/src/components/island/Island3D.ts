/**
 * Island3D —— 岛屿工作空间渲染器（《我们的岛》式卡通海岛 + 昼夜光照 + 交互）。
 *
 * 职责边界（与桌宠 PetSvg 同一套工程范式）：
 * - **只做渲染**：几何由 islandBuilders 构建，动画数学由 islandSim 提供（纯函数、已单测），
 *   本类只做「数值 → 网格」映射与 WebGL 生命周期。
 * - **诚实渲染**：构造前校验 Spec，坏 Spec 直接抛错；无 WebGL 由上层走 2D 降级（本类不假装可用）。
 * - **性能自理**：质量档预算（阴影/抗锯齿/采样率/光源数/帧率上限/装饰量）、
 *   不可见即暂停、增量 diff（建筑按 id 复用，不重建场景）、材质共享。
 * - **主题联动**：UI 语义色（accent/ok/warn/danger/info）从 CSS 变量读入，
 *   道路/货物/选中环随主题变化，建筑配色仍来自 Spec 调色板。
 */

import * as THREE from 'three'
import { ISLAND_KIND_META } from './islandContract'
import type { IslandModelSpec, IslandQualityTier } from './islandSpec'
import type { IslandEntity, IslandWorld } from './islandWorld'
import { plotFor } from './islandWorld'
import {
  computeIslandFrame,
  createIslandCamera,
  dampCamera,
  fillDirection,
  focusCamera,
  hashUnit,
  orbitCamera,
  panCamera,
  waterTintAt,
  waterWaveAt,
  zoomCamera,
  type IslandFrame,
  type IslandSkyState,
  type IslandCameraCommand,
  type OrbitCamera,
} from './islandSim'
import {
  buildBuilding,
  buildClouds,
  buildProps,
  buildRoad,
  buildTerrain,
  buildWater,
  createIslandMaterials,
  roadCurve,
  type IslandBuildingHandle,
  type IslandKeepOut,
  type IslandMaterialSet,
  type IslandWaterHandle,
} from './islandBuilders'
import {
  applyConstructionSnapshot,
  buildConstructionPlot,
  updateConstructionAnimation,
  type ConstructionPlotHandle,
} from './constructionPlotBuilders'
import { createRadialGlowTexture } from '../three/glowTexture'

/** UI 语义色（3D 侧取色口径，来源于主题 CSS 变量） */
export interface IslandThemeTokens {
  accent: number
  ok: number
  warn: number
  danger: number
  info: number
}

/** 稳定回退色（仅在 CSS 变量解析失败时使用；与主题 token 同色系） */
const FALLBACK_TOKENS: IslandThemeTokens = {
  accent: 0x6aa9ff,
  ok: 0x6cc78a,
  warn: 0xe8b45f,
  danger: 0xe4736b,
  info: 0x7fc4e8,
}

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
      return (Math.round(r) << 16) | (Math.round(g) << 8) | Math.round(b)
    }
  }
  return fallback
}

/** 从 document 计算样式读取主题语义色（主题切换后由视图再次调用并 setTheme） */
export function readIslandThemeTokens(): IslandThemeTokens {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return FALLBACK_TOKENS
  const style = getComputedStyle(document.documentElement)
  const read = (name: string, fallback: number) => parseCssColor(style.getPropertyValue(name), fallback)
  return {
    accent: read('--accent', FALLBACK_TOKENS.accent),
    ok: read('--ok', FALLBACK_TOKENS.ok),
    warn: read('--warn', FALLBACK_TOKENS.warn),
    danger: read('--danger', FALLBACK_TOKENS.danger),
    info: read('--info', FALLBACK_TOKENS.info),
  }
}

export interface IslandLabelAnchor {
  id: string
  x: number
  y: number
  visible: boolean
  selected: boolean
}

/** HTML 标签胶囊的估算尺寸（与 `.island-label` 的 11px 字号、2×8px 横向内边距对齐）。
 *
 *  投影层每 100ms 跑一次，不能去量 DOM（会触发重排）；用字宽估算即可——
 *  宁可早一点隐藏，也不要让胶囊被画布边缘切掉一半（真机截图里就出现过半截药丸）。 */
export function estimateLabelBox(title: string): { width: number; height: number } {
  let width = 16
  for (const ch of title) {
    width += ((ch.codePointAt(0) ?? 0) > 0x2e80 ? 11 : 6.2)
  }
  return { width: Math.min(width, 220), height: 22 }
}

/** 锚点是否能让整枚标签落在画布内。
 *
 *  标签是 `translate(-50%, -100%)` 的胶囊：底边贴锚点、水平居中，
 *  因此要求「上沿 ≥ 0、下沿 ≤ 高、左右都不越界」。 */
export function anchorFitsCanvas(
  px: number,
  py: number,
  width: number,
  height: number,
  box: LabelBox,
): boolean {
  if (width <= 0 || height <= 0) return false
  const half = box.width / 2
  return px - half >= 0 && px + half <= width && py - box.height >= 0 && py <= height - 2
}

export interface LabelBox {
  width: number
  height: number
}

/** 同屏标签的竖向避让：`y` 是胶囊底边，按 y 从上到下依次排开。
 *
 *  真机实测：`session`（主堡）与 `construction:workshop-tower` 的胶囊在默认视角
 *  重叠 5.3px，把主堡名字遮掉约四分之一。锚点间距太近时**只下移**下方那枚，
 *  x 保持不变——宁可标签略微离开建筑顶部，也不要两枚名字叠在一起。
 *  下移后若越出画布底部，则该枚退回隐藏（宁缺勿假）。 */
export function separateLabelAnchors(
  anchors: readonly IslandLabelAnchor[],
  boxes: ReadonlyMap<string, LabelBox>,
  width: number,
  height: number,
  gap = 2,
): IslandLabelAnchor[] {
  const visible = anchors
    .filter((anchor) => anchor.visible)
    .sort((a, b) => a.y - b.y || a.x - b.x)
  let floor = Number.NEGATIVE_INFINITY
  const adjusted: IslandLabelAnchor[] = []
  for (const anchor of visible) {
    const box = boxes.get(anchor.id) ?? { width: 0, height: 22 }
    const y = Math.max(anchor.y, floor + box.height + gap)
    const stillFits = y <= height - 2 && anchorFitsCanvas(anchor.x, y, width, height, box)
    adjusted.push({ ...anchor, y, visible: stillFits })
    floor = y
  }
  // 未参与避让（不可见）的锚点原样保留，便于上层按 id 稳定渲染
  const hidden = anchors.filter((anchor) => !anchor.visible)
  return [...adjusted, ...hidden]
}

export interface Island3DOptions {
  container: HTMLElement
  spec: IslandModelSpec
  quality: IslandQualityTier
  theme?: IslandThemeTokens
  onSelect?: (entityId: string | null) => void
  onHover?: (entityId: string | null) => void
  /** 每 100ms 一次的标签投影（HTML 层跟随 3D 位置） */
  onLabels?: (anchors: IslandLabelAnchor[]) => void
  /** 标签显示名（缺省用 entity.title）。视图层用它做本地化 / 占位名替换，
   *  投影层则用同一份文案估算胶囊宽度——两处必须同源，否则估算宽度与实际渲染不一致，
   *  边缘标签会重新出现「差几像素被裁一半」。 */
  labelTitle?: (entity: IslandEntity) => string
  /** WebGL 上下文丢失（上层应切到 2D 降级并提示） */
  onContextLost?: () => void
  /** 施工地块落点（让装饰道具别在脚手架底下种树；缺省为空） */
  constructionKeepOuts?: readonly { x: number; z: number; radius: number }[]
}

interface EntityHandle {
  entity: IslandEntity
  root: THREE.Group
  building: IslandBuildingHandle
  /** 建筑顶部 y（标签锚点） */
  topY: number
  /** 施工地块句柄（仅 kind==='construction'） */
  construction?: ConstructionPlotHandle
}

interface LinkHandle {
  active: boolean
  road: THREE.Mesh
  glow: THREE.Mesh
  glowMaterial: THREE.MeshToonMaterial
  cargo: THREE.Mesh
  curve: THREE.QuadraticBezierCurve3
}

export class Island3D {
  private readonly container: HTMLElement
  private readonly spec: IslandModelSpec
  private readonly quality: IslandQualityTier
  /** 基准纵向视角（横向视野由它 + 宽高比推导） */
  private readonly baseFov = 46
  private readonly budget
  private theme: IslandThemeTokens
  private readonly onSelect?: (id: string | null) => void
  private readonly onHover?: (id: string | null) => void
  private readonly onLabels?: (anchors: IslandLabelAnchor[]) => void
  private readonly labelTitle?: (entity: IslandEntity) => string
  private readonly onContextLost?: () => void

  private renderer: THREE.WebGLRenderer
  private scene: THREE.Scene
  private camera: THREE.PerspectiveCamera
  private materials: IslandMaterialSet
  private water: IslandWaterHandle
  private waterPosition: THREE.BufferAttribute
  private waterBase: Float32Array
  private sky: THREE.Mesh
  private skyTexture: THREE.DataTexture
  private sun: THREE.DirectionalLight
  private fill: THREE.DirectionalLight
  private hemi: THREE.HemisphereLight
  private lightPool: THREE.PointLight[] = []
  private entityGroup = new THREE.Group()
  private roadGroup = new THREE.Group()
  private cargoGroup = new THREE.Group()
  private clouds: THREE.Group
  private cloudMaterial: THREE.MeshBasicMaterial
  /** 柔光贴图（日月圆盘 + 灯火光晕 + 星星共用） */
  private glowTexture: THREE.CanvasTexture
  private sunDisk: THREE.Sprite
  private stars: THREE.Points
  private starMaterial: THREE.PointsMaterial
  private halos: THREE.Sprite[] = []
  private selectionRing: THREE.Mesh
  private selectionMaterial: THREE.MeshBasicMaterial

  private entities = new Map<string, EntityHandle>()
  private links = new Map<string, LinkHandle>()
  private world: IslandWorld | null = null
  /** 施工地块落点（让装饰道具别在脚手架底下种树；由视图在建场景前注入） */
  private constructionKeepOuts: IslandKeepOut[] = []

  private cameraState: OrbitCamera
  private cameraGoal: OrbitCamera
  private lastFrameAt = 0
  private elapsed = 0
  private timeOfDay: number
  private autoDayNight = true
  private reducedMotion = false
  private paused = false
  private alive = true
  private rafId = 0
  private lastRender = 0
  private lastSkyTexture = 0
  private lastLabels = 0
  private lastHoverCheck = 0
  private hoveredId: string | null = null
  private selectedId: string | null = null

  // 指针交互状态
  private pointerMode: 'none' | 'orbit' | 'pan' = 'none'
  private pointerStart = { x: 0, y: 0, time: 0 }
  private pointerLast = { x: 0, y: 0 }
  private readonly raycaster = new THREE.Raycaster()
  private readonly pointerNdc = new THREE.Vector2()

  private resizeObserver?: ResizeObserver
  private intersectionObserver?: IntersectionObserver
  private onVisibilityChange?: () => void
  private onMotionChange?: (event: MediaQueryListEvent) => void
  private motionQuery?: MediaQueryList

  constructor(opts: Island3DOptions) {
    this.container = opts.container
    this.spec = opts.spec
    this.quality = opts.quality
    this.theme = opts.theme ?? FALLBACK_TOKENS
    this.budget = this.spec.quality[this.quality]
    this.onSelect = opts.onSelect
    this.onHover = opts.onHover
    this.onLabels = opts.onLabels
    this.labelTitle = opts.labelTitle
    this.onContextLost = opts.onContextLost
    this.constructionKeepOuts = (opts.constructionKeepOuts ?? []).map((p) => ({ x: p.x, z: p.z, radius: p.radius }))

    const width = Math.max(1, this.container.clientWidth)
    const height = Math.max(1, this.container.clientHeight)

    this.renderer = new THREE.WebGLRenderer({
      antialias: this.budget.antialias,
      powerPreference: this.budget.powerPreference,
    })
    // 卡通渲染：不做电影曲线映射，保证调色板颜色所见即所得（与桌宠一致）
    this.renderer.toneMapping = THREE.NoToneMapping
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, this.budget.maxPixelRatio))
    this.renderer.setSize(width, height)
    this.renderer.shadowMap.enabled = this.budget.shadows
    // PCFSoftShadowMap 在 three r18x 已弃用（内部会回退成 PCFShadowMap 并打弃用警告），
    // 直接写 PCFShadowMap：视觉等价、控制台干净。
    this.renderer.shadowMap.type = THREE.PCFShadowMap
    this.renderer.domElement.style.cssText = 'width:100%;height:100%;display:block;touch-action:none;cursor:grab;'
    this.renderer.domElement.addEventListener('webglcontextlost', this.handleContextLost)
    this.container.appendChild(this.renderer.domElement)

    this.scene = new THREE.Scene()
    this.scene.background = new THREE.Color(this.spec.palette.water)
    // 注意 THREE.Fog 的签名是 (color, near, far)：少传 color 会让 near/far 整体错位
    this.scene.fog = new THREE.Fog(this.spec.palette.cloud, this.spec.lighting.fogNear, this.spec.lighting.fogFar)

    this.camera = new THREE.PerspectiveCamera(this.baseFov, width / height, 0.5, this.spec.water.radius * 4)
    this.applyCameraFraming(width / height)
    this.cameraState = createIslandCamera(this.spec)
    this.cameraGoal = { ...this.cameraState }
    this.timeOfDay = this.spec.lighting.dayLength * 0.3

    this.materials = createIslandMaterials()
    const detail = this.budget.terrainDetail
    this.scene.add(buildTerrain(this.spec, this.materials, detail))

    this.water = buildWater(this.spec, this.budget.waterSegments)
    this.waterPosition = this.water.mesh.geometry.attributes.position as THREE.BufferAttribute
    this.waterBase = this.water.basePositions
    this.scene.add(this.water.mesh, this.water.foam)

    // 装饰物预先避让所有建筑槽位（布局稳定 → 装饰不会与建筑打架，也不会随数据变化跳动）
    this.scene.add(buildProps(this.spec, this.materials, this.budget.props, this.keepOutZones()))
    const clouds = buildClouds(this.spec, this.budget.props)
    this.clouds = clouds.group
    this.cloudMaterial = clouds.material
    this.scene.add(this.clouds)

    // ── 唯美光效层（不用后处理 bloom：柔光贴图 + 叠加混合，零管线风险）──
    this.glowTexture = createRadialGlowTexture()
    this.sunDisk = new THREE.Sprite(new THREE.SpriteMaterial({
      map: this.glowTexture,
      transparent: true,
      depthWrite: false,
      fog: false,
      blending: THREE.AdditiveBlending,
      opacity: 0.8,
    }))
    this.sunDisk.scale.setScalar(this.spec.terrain.radius * 0.62)
    this.scene.add(this.sunDisk)

    const starField = this.buildStarField()
    this.stars = starField.points
    this.starMaterial = starField.material
    this.scene.add(this.stars)

    for (let i = 0; i < Math.max(1, this.budget.lights); i++) {
      const halo = new THREE.Sprite(new THREE.SpriteMaterial({
        map: this.glowTexture,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        opacity: 0,
        fog: false,
      }))
      halo.scale.setScalar(this.spec.terrain.radius * 0.12)
      this.halos.push(halo)
      this.scene.add(halo)
    }

    // 天空穹顶（渐变贴图，随昼夜更新；走标准材质管线以保持色彩一致）
    this.skyTexture = new THREE.DataTexture(new Uint8Array(2 * 64 * 4), 2, 64, THREE.RGBAFormat)
    this.skyTexture.minFilter = THREE.LinearFilter
    this.skyTexture.magFilter = THREE.LinearFilter
    this.sky = new THREE.Mesh(
      // 半径贴近相机远平面：太小会让穹顶边界在画面上方露出一条生硬的弧线
      new THREE.SphereGeometry(this.spec.water.radius * 3.7, 32, 20),
      // 天空穹顶必须不受雾影响：否则远处穹顶会被雾色染出一条横贯画面的亮带
      new THREE.MeshBasicMaterial({ map: this.skyTexture, side: THREE.BackSide, fog: false, depthWrite: false }),
    )
    this.scene.add(this.sky)

    this.hemi = new THREE.HemisphereLight(0xffffff, this.spec.palette.grass, 1)
    this.scene.add(this.hemi)
    this.sun = new THREE.DirectionalLight(0xffffff, 1)
    this.sun.castShadow = this.budget.shadows
    if (this.budget.shadows) {
      const shadowSpan = this.spec.terrain.radius * 1.35
      this.sun.shadow.mapSize.set(2048, 2048)
      this.sun.shadow.camera.near = 1
      this.sun.shadow.camera.far = this.spec.lighting.sunRadius * 2.6
      this.sun.shadow.camera.left = -shadowSpan
      this.sun.shadow.camera.right = shadowSpan
      this.sun.shadow.camera.top = shadowSpan
      this.sun.shadow.camera.bottom = -shadowSpan
      this.sun.shadow.bias = -0.0009
      this.sun.shadow.radius = 3
    }
    this.scene.add(this.sun)
    // 逆光补光：从太阳对侧斜打，托住岛缘侧壁（否则会压出一圈近黑描边）。
    // 不投影——补光投影只会产生与主光方向矛盾的第二重影子。
    this.fill = new THREE.DirectionalLight(0xffffff, 0)
    this.fill.castShadow = false
    this.scene.add(this.fill)
    for (let i = 0; i < this.budget.lights; i++) {
      const light = new THREE.PointLight(this.spec.lighting.lanternColor, 0, this.spec.terrain.radius * 0.9, 2)
      this.lightPool.push(light)
      this.scene.add(light)
    }

    this.selectionMaterial = new THREE.MeshBasicMaterial({
      color: this.theme.accent,
      transparent: true,
      opacity: 0,
      depthWrite: false,
    })
    this.selectionRing = new THREE.Mesh(new THREE.TorusGeometry(1.9, 0.07, 6, 40), this.selectionMaterial)
    this.selectionRing.rotation.x = Math.PI / 2
    this.selectionRing.position.y = 0.06
    this.scene.add(this.selectionRing)

    this.scene.add(this.entityGroup, this.roadGroup, this.cargoGroup)

    this.setupObservers()
    this.attachPointerHandlers()
    this.applySky({ ...this.emptySky() })
    this.tick()
  }

  /* ── 对外 API ── */

  setWorld(world: IslandWorld): void {
    if (!this.alive) return
    this.world = world
    const seen = new Set<string>()
    for (const entity of world.entities) {
      seen.add(entity.id)
      const handle = this.entities.get(entity.id)
      if (handle && handle.entity.kind === entity.kind && handle.entity.flavor === entity.flavor) {
        // 形态未变 → 只换数据（几何复用，避免每帧/每次事件重建场景）
        handle.entity = entity
        handle.root.position.set(entity.plot.x, 0, entity.plot.z)
        handle.root.rotation.y = entity.plot.rotation
        continue
      }
      if (handle) this.removeEntity(entity.id)
      this.addEntity(entity)
    }
    for (const id of [...this.entities.keys()]) {
      if (!seen.has(id)) this.removeEntity(id)
    }
    this.syncLinks(world)
  }

  /** 手动接管昼夜（0..1 为一天中的归一化进度） */
  setDayProgress(progress: number): void {
    this.autoDayNight = false
    this.timeOfDay = progress * this.spec.lighting.dayLength
  }

  setAutoDayNight(enabled: boolean): void {
    if (enabled) {
      // 从当前时刻继续，避免切回自动时跳变
      this.elapsed = this.timeOfDay
      this.lastFrameAt = performance.now()
    } else {
      this.timeOfDay = (this.elapsed % this.spec.lighting.dayLength + this.spec.lighting.dayLength) % this.spec.lighting.dayLength
    }
    this.autoDayNight = enabled
  }

  isAutoDayNight(): boolean {
    return this.autoDayNight
  }

  setTheme(theme: IslandThemeTokens): void {
    this.theme = theme
    this.selectionMaterial.color.setHex(theme.accent)
    for (const link of this.links.values()) {
      link.glowMaterial.color.setHex(theme.accent)
      link.glowMaterial.emissive.setHex(theme.accent)
    }
  }

  select(entityId: string | null): void {
    this.selectedId = entityId
    if (!entityId) {
      this.selectionMaterial.opacity = 0
      return
    }
    const handle = this.entities.get(entityId)
    if (handle) {
      this.selectionRing.position.set(handle.entity.plot.x, 0.06, handle.entity.plot.z)
      this.selectionMaterial.opacity = 0.85
    }
  }

  hover(entityId: string | null): void {
    if (this.hoveredId === entityId) return
    this.hoveredId = entityId
    this.renderer.domElement.style.cursor = entityId ? 'pointer' : 'grab'
    this.onHover?.(entityId)
  }

  focusEntity(entityId: string): void {
    const handle = this.entities.get(entityId)
    if (!handle) return
    this.cameraGoal = focusCamera(this.cameraGoal, { x: handle.entity.plot.x, z: handle.entity.plot.z }, this.spec)
  }

  resetView(): void {
    this.cameraGoal = createIslandCamera(this.spec)
  }

  /**
   * 键盘导航：执行一条相机命令。
   * 与指针交互共用同一套 `cameraGoal` + 阻尼收敛路径，因此键盘与拖拽的手感一致，
   * 且同样受 `islandCameraLimits` 约束（不会翻面/穿地/飞出岛域）。
   */
  applyCameraCommand(cmd: IslandCameraCommand): void {
    switch (cmd.kind) {
      case 'orbit':
        this.cameraGoal = orbitCamera(this.cameraGoal, cmd.yaw, cmd.pitch, this.spec)
        break
      case 'zoom':
        this.cameraGoal = zoomCamera(this.cameraGoal, cmd.delta, this.spec)
        break
      case 'pan':
        this.cameraGoal = panCamera(this.cameraGoal, cmd.dx, cmd.dy, this.spec)
        break
      case 'reset':
        this.resetView()
        break
    }
  }

  setPaused(paused: boolean): void {
    this.paused = paused
    if (!paused) {
      // 恢复时重置计时，避免一次巨大的 dt
      this.lastFrameAt = performance.now()
      this.lastRender = 0
    }
  }

  dispose(): void {
    this.alive = false
    cancelAnimationFrame(this.rafId)
    this.resizeObserver?.disconnect()
    this.intersectionObserver?.disconnect()
    if (this.onVisibilityChange) document.removeEventListener('visibilitychange', this.onVisibilityChange)
    if (this.onMotionChange && this.motionQuery) this.motionQuery.removeEventListener('change', this.onMotionChange)
    this.detachPointerHandlers()
    this.renderer.domElement.removeEventListener('webglcontextlost', this.handleContextLost)

    for (const id of [...this.entities.keys()]) this.removeEntity(id)
    for (const id of [...this.links.keys()]) this.removeLink(id)
    this.scene.traverse((object) => {
      if (object instanceof THREE.Mesh) {
        object.geometry.dispose()
        const material = object.material
        if (Array.isArray(material)) material.forEach((m) => m.dispose())
        else material.dispose()
      }
    })
    this.materials.dispose()
    this.skyTexture.dispose()
    this.selectionMaterial.dispose()
    // Sprite / Points 不是 Mesh，需显式回收（traverse 只处理 Mesh）
    this.glowTexture.dispose()
    ;(this.sunDisk.material as THREE.SpriteMaterial).dispose()
    for (const halo of this.halos) (halo.material as THREE.SpriteMaterial).dispose()
    this.starMaterial.dispose()
    this.stars.geometry.dispose()
    this.cloudMaterial.dispose()
    this.water.foamMaterial.dispose()
    this.renderer.dispose()
    if (this.renderer.domElement.parentNode) {
      this.renderer.domElement.parentNode.removeChild(this.renderer.domElement)
    }
  }

  /* ── 场景内容 ── */

  private keepOutZones(): IslandKeepOut[] {
    const zones: IslandKeepOut[] = [{ x: 0, z: 0, radius: this.spec.layout.coreRadius }]
    for (const kind of ['workshop', 'outpost', 'depot', 'beacon', 'pier', 'athenaeum'] as const) {
      const template = this.spec.buildings[kind]
      const pad = Math.max(template.width, template.depth) * 0.6 + 0.7
      for (let index = 0; index < ISLAND_KIND_META[kind].maxInstances; index++) {
        const plot = plotFor(kind, index, this.spec)
        zones.push({ x: plot.x, z: plot.z, radius: pad })
      }
    }
    for (const pad of this.constructionKeepOuts) zones.push(pad)
    return zones
  }

  private addEntity(entity: IslandEntity): void {
    // 施工地块：逐工序真实增删部件（不用通用 buildBuilding 拉伸盒子）
    if (entity.kind === 'construction' && entity.construction) {
      const cHandle = buildConstructionPlot(entity.construction.config, this.materials)
      const root = new THREE.Group()
      root.position.set(entity.plot.x, 0, entity.plot.z)
      root.rotation.y = entity.plot.rotation
      root.add(cHandle.group)
      root.userData.entityId = entity.id
      cHandle.group.userData.entityId = entity.id
      this.entityGroup.add(root)
      const topY = entity.construction.config.building.totalFloors * entity.construction.config.building.floorHeight + 0.4
      const emptyBuilding: IslandBuildingHandle = { group: cHandle.group, glowMaterials: [], spinners: [], planks: [], smoke: [] }
      this.entities.set(entity.id, { entity, root, building: emptyBuilding, topY, construction: cHandle })
      applyConstructionSnapshot(cHandle, entity.construction.snapshot)
      return
    }
    const template = this.spec.buildings[entity.kind]
    const building = buildBuilding(entity.kind, template, this.spec, this.materials, entity.flavor, this.budget)
    const root = new THREE.Group()
    root.position.set(entity.plot.x, 0, entity.plot.z)
    root.rotation.y = entity.plot.rotation
    root.add(building.group)
    root.userData.entityId = entity.id
    building.group.userData.entityId = entity.id
    this.entityGroup.add(root)
    const topY = entity.kind === 'pier'
      ? 1.8
      : template.height + template.roofHeight + (entity.kind === 'core' ? 1.2 : 0.4)
    this.entities.set(entity.id, { entity, root, building, topY })
  }

  private removeEntity(id: string): void {
    const handle = this.entities.get(id)
    if (!handle) return
    this.entityGroup.remove(handle.root)
    handle.root.traverse((object) => {
      if (object instanceof THREE.Mesh) {
        object.geometry.dispose()
        const material = object.material
        // 共享材质由 materials.dispose() 统一回收；只销毁建筑私有 clone
        if (handle.building.glowMaterials.includes(material as THREE.MeshToonMaterial)) {
          ;(material as THREE.Material).dispose()
        }
      }
    })
    this.entities.delete(id)
  }

  private syncLinks(world: IslandWorld): void {
    const seen = new Set<string>()
    for (const link of world.links) {
      seen.add(link.id)
      const existing = this.links.get(link.id)
      if (existing) {
        existing.active = link.active
        continue
      }
      const from = { x: 0, z: 0 }
      const target = this.entities.get(link.to)
      if (!target) continue
      const curve = roadCurve(
        new THREE.Vector3(from.x, 0, from.z),
        new THREE.Vector3(target.entity.plot.x, entityGroundY(target.entity), target.entity.plot.z),
      )
      const road = buildRoad(curve, this.materials, this.spec.palette.wood, 0.17)
      const glowMaterial = this.materials.translucent(this.theme.accent, 0.5).clone()
      glowMaterial.emissive = new THREE.Color(this.theme.accent)
      glowMaterial.emissiveIntensity = 0
      const glow = new THREE.Mesh(new THREE.TubeGeometry(curve, 16, 0.06, 4, false), glowMaterial)
      const cargo = new THREE.Mesh(
        new THREE.BoxGeometry(0.26, 0.26, 0.26),
        new THREE.MeshToonMaterial({ color: this.spec.palette.cloth, emissive: new THREE.Color(this.theme.accent), emissiveIntensity: 0.9 }),
      )
      cargo.visible = false
      this.roadGroup.add(road, glow)
      this.cargoGroup.add(cargo)
      this.links.set(link.id, { active: link.active, road, glow, glowMaterial, cargo, curve })
    }
    for (const id of [...this.links.keys()]) {
      if (!seen.has(id)) this.removeLink(id)
    }
  }

  private removeLink(id: string): void {
    const handle = this.links.get(id)
    if (!handle) return
    this.roadGroup.remove(handle.road, handle.glow)
    this.cargoGroup.remove(handle.cargo)
    handle.road.geometry.dispose()
    handle.glow.geometry.dispose()
    handle.glowMaterial.dispose()
    handle.cargo.geometry.dispose()
    ;(handle.cargo.material as THREE.Material).dispose()
    this.links.delete(id)
  }

  /* ── 帧循环 ── */

  /** 帧间隔（秒）。用 `performance.now()` 而不是 `THREE.Clock`：r18x 起 Clock 已弃用。 */
  private takeDelta(): number {
    const now = performance.now()
    const dt = this.lastFrameAt === 0 ? 0 : (now - this.lastFrameAt) / 1000
    this.lastFrameAt = now
    return dt
  }

  private tick = (): void => {
    if (!this.alive) return
    this.rafId = requestAnimationFrame(this.tick)
    const dt = Math.min(this.takeDelta(), 0.05)
    if (this.paused) return
    this.elapsed += dt

    const now = performance.now()
    const frameInterval = 1000 / this.budget.targetFps
    if (now - this.lastRender < frameInterval - 2) return
    this.lastRender = now

    const dayTime = this.autoDayNight ? this.elapsed + this.timeOfDay : this.timeOfDay
    const frame = this.world
      ? computeIslandFrame(this.spec, this.world, this.elapsed, dayTime)
      : null
    if (frame) {
      this.applySky(frame.sky, now)
      this.applyEntities(frame, dt, now)
      this.applyTraffic(frame, dt)
    }
    this.updateWater()
    this.updateClouds(dt)
    this.updateCamera(dt)
    this.renderer.render(this.scene, this.camera)
    this.emitLabels(now)
  }

  private applySky(sky: IslandSkyState, now = 0): void {
    const L = this.spec.lighting
    const dir = sky.sunDirection
    this.sun.position.set(dir.x * L.sunRadius, Math.max(0.6, dir.y * L.sunRadius), dir.z * L.sunRadius)
    this.sun.color.setHex(sky.sunColor)
    this.sun.intensity = sky.sunIntensity
    this.sun.castShadow = this.budget.shadows && sky.dayFactor > 0.12
    // 补光：方向取太阳水平对侧；白天满强度、夜里保留约 1/4（免得岛缘再次压黑）
    const fdir = fillDirection(sky)
    this.fill.position.set(fdir.x * L.sunRadius, fdir.y * L.sunRadius, fdir.z * L.sunRadius)
    this.fill.color.setHex(mixHex(sky.skyTop, this.spec.palette.cloud, 0.5))
    this.fill.intensity = L.fillIntensity * (0.25 + 0.75 * sky.dayFactor)
    this.hemi.color.setHex(sky.skyTop)
    this.hemi.groundColor.setHex(this.spec.palette.grass)
    this.hemi.intensity = sky.ambientIntensity
    ;(this.scene.fog as THREE.Fog).color.setHex(sky.fogColor)
    ;(this.scene.background as THREE.Color).setHex(sky.skyBottom)
    // 水体色阶已由顶点色承载（浅滩→远海径向渐变，见 islandBuilders.buildWater），
    // 这里只做**昼夜色温染色**（waterTintAt：白天恰好中性白，不干扰色阶）。
    this.water.material.color.setHex(waterTintAt(this.spec, sky.nightFactor))
    // 日月圆盘：夜晚取 |y| 变成月亮高度（同一位、不同色温），白天暖白、夜里月光
    const diskMaterial = this.sunDisk.material as THREE.SpriteMaterial
    const diskRadius = L.sunRadius * 1.15
    this.sunDisk.position.set(dir.x * diskRadius, Math.abs(dir.y) * diskRadius + 1.1, dir.z * diskRadius)
    diskMaterial.color.setHex(sky.sunColor)
    diskMaterial.opacity = 0.22 + 0.58 * (1 - sky.nightFactor)
    // 星空：只在夜里浮现（两次幂让暮色时几乎不可见，入夜后清晰）
    this.starMaterial.opacity = 0.95 * sky.nightFactor * sky.nightFactor
    // 云：白天纯白 → 黄昏被地平线色染暖 → 夜里压暗（不再是一团午夜白）
    this.cloudMaterial.color.setHex(mixHex(this.spec.palette.cloud, sky.skyBottom, 1 - sky.dayFactor))
    this.cloudMaterial.opacity = 0.58 + 0.14 * sky.dayFactor
    if (now - this.lastSkyTexture > 220) {
      this.lastSkyTexture = now
      this.updateSkyTexture(sky)
    }
  }

  /** 星空（集中在相机俯视时真正可见的仰角带；按档位预算减量） */
  private buildStarField(): { points: THREE.Points; material: THREE.PointsMaterial } {
    const count = this.budget.stars
    const radius = this.spec.water.radius * 1.15
    const positions = new Float32Array(count * 3)
    for (let i = 0; i < count; i++) {
      const angle = hashUnit(i * 3 + 11) * Math.PI * 2
      // 仰角 1°~32°：贴近地平线 —— 俯视轨道相机只在这段天空里能看到星（再高就出画面）
      const elevation = 0.02 + hashUnit(i * 7 + 5) * 0.54
      const horizontal = Math.cos(elevation)
      positions[i * 3] = Math.cos(angle) * horizontal * radius
      positions[i * 3 + 1] = Math.sin(elevation) * radius
      positions[i * 3 + 2] = Math.sin(angle) * horizontal * radius
    }
    const geometry = new THREE.BufferGeometry()
    geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    const material = new THREE.PointsMaterial({
      // 不用柔光贴图：星点需要「锐利的小亮点」，贴图会把它糊成几乎看不见的微光
      size: this.budget.starSize,
      sizeAttenuation: false,
      transparent: true,
      opacity: 0,
      depthWrite: false,
      fog: false,
      blending: THREE.AdditiveBlending,
    })
    return { points: new THREE.Points(geometry, material), material }
  }

  private updateSkyTexture(sky: IslandSkyState): void {
    const data = this.skyTexture.image.data as Uint8Array
    for (let row = 0; row < 64; row++) {
      // v=0 → 地平线（云层/雾色），v=1 → 天顶
      const t = row / 63
      const eased = t * t * (3 - 2 * t)
      const color = mixHex(sky.skyBottom, sky.skyTop, eased)
      data[row * 8 + 0] = (color >> 16) & 0xff
      data[row * 8 + 1] = (color >> 8) & 0xff
      data[row * 8 + 2] = color & 0xff
      data[row * 8 + 3] = 0xff
      data[row * 8 + 4] = (color >> 16) & 0xff
      data[row * 8 + 5] = (color >> 8) & 0xff
      data[row * 8 + 6] = color & 0xff
      data[row * 8 + 7] = 0xff
    }
    this.skyTexture.needsUpdate = true
  }

  private applyEntities(frame: IslandFrame, dt: number, now: number): void {
    const visuals = new Map(frame.entities.map((visual) => [visual.id, visual]))
    const litCandidates: { handle: EntityHandle; intensity: number; color: number }[] = []

    for (const [id, handle] of this.entities) {
      const visual = visuals.get(id)
      if (!visual) continue
      const entity = handle.entity
      // 施工地块：整层增删部件 + 塔吊/工人/车辆循环；不做活体建筑的呼吸/浮沉
      if (handle.construction && entity.construction) {
        applyConstructionSnapshot(handle.construction, entity.construction.snapshot)
        updateConstructionAnimation(handle.construction, this.elapsed)
        handle.topY = entity.construction.config.building.floorHeight * entity.construction.snapshot.visibleFloors + 0.4
        continue
      }
      const nightBoost = 1 - frame.sky.nightFactor * 0.25
      for (const material of handle.building.glowMaterials) {
        material.emissiveIntensity = visual.glow * this.spec.lighting.lanternIntensity * nightBoost
      }
      // 活动建筑轻微「呼吸」，错误建筑高频抖动（一眼看出哪里出问题了）
      const pulseScale = this.reducedMotion ? 1 : 1 + (visual.pulse - 0.5) * (entity.status === 'active' ? 0.035 : 0.012)
      handle.root.scale.setScalar(pulseScale)
      handle.root.position.y = this.reducedMotion ? 0 : visual.bob

      // 栈桥：一步一块铺板
      if (handle.building.planks.length > 0 && entity.progress !== undefined) {
        const visibleCount = Math.ceil(entity.progress * handle.building.planks.length)
        handle.building.planks.forEach((plank, index) => {
          plank.visible = index < visibleCount
        })
      }
      // 仓库货箱随装载率显隐
      const crates = handle.building.group.children.filter((child) => child.userData.plank !== undefined)
      if (crates.length > 0 && entity.progress !== undefined) {
        const loaded = Math.ceil(entity.progress * crates.length)
        crates.forEach((crate, index) => {
          crate.visible = index < loaded
        })
      }
      // 水车/风车/记忆塔光晕环（绕自身轴自转；水平环用 spinAxis='y'）
      for (const spinner of handle.building.spinners) {
        const speed = dt * (entity.status === 'active' ? 2.2 : 0.35)
        if (spinner.userData.spinAxis === 'y') spinner.rotation.y += speed
        else spinner.rotation.x += speed
      }
      // 灯塔光束：夜晚 + 活动时扫动（枢轴 Group 绕 Y 旋转，锥体自身沿本地 X 张开）
      if (handle.building.beam) {
        const beamMesh = handle.building.beam.children[0] as THREE.Mesh | undefined
        const beamMaterial = beamMesh?.material as THREE.MeshToonMaterial | undefined
        if (beamMaterial) beamMaterial.opacity = (0.04 + 0.24 * visual.glow) * frame.sky.beamIntensity
        handle.building.beam.rotation.y += dt * (0.35 + visual.glow * 0.9)
      }
      // 烟柱
      handle.building.smoke.forEach((puff, index) => {
        const material = (puff as THREE.Mesh).material as THREE.MeshBasicMaterial
        material.opacity = visual.glow * 0.32 * Math.max(0, 1 - index * 0.25)
        if (!this.reducedMotion) {
          puff.position.y += dt * 0.25 * (0.5 + visual.glow)
          if (puff.position.y > this.spec.buildings.workshop.height + 3) {
            puff.position.y = this.spec.buildings.workshop.height + 1
          }
        }
      })

      if (visual.lantern && ISLAND_KIND_META[entity.kind].glow) {
        litCandidates.push({
          handle,
          intensity: visual.glow,
          color: entity.status === 'error' ? this.theme.danger : this.spec.lighting.lanternColor,
        })
      }
    }

    // 点光源预算：按亮度取前 N 个（低质量档少灯，省片元开销）
    litCandidates.sort((a, b) => b.intensity - a.intensity)
    this.lightPool.forEach((light, index) => {
      const candidate = litCandidates[index]
      const halo = this.halos[index]
      if (!candidate) {
        light.intensity = 0
        if (halo) (halo.material as THREE.SpriteMaterial).opacity = 0
        return
      }
      const entity = candidate.handle.entity
      light.position.set(entity.plot.x, candidate.handle.topY * 0.5, entity.plot.z)
      light.color.setHex(candidate.color)
      light.intensity = candidate.intensity * this.budget.lightGain
      // 灯火光晕（叠加柔光）：只在夜里与建筑活动时可见 —— 「温润灯火」的来源
      if (halo) {
        const haloMaterial = halo.material as THREE.SpriteMaterial
        halo.position.set(entity.plot.x, candidate.handle.topY * 0.62, entity.plot.z)
        haloMaterial.color.setHex(candidate.color)
        haloMaterial.opacity = Math.min(0.46, candidate.intensity * (0.08 + 0.4 * frame.sky.nightFactor))
        halo.scale.setScalar(this.spec.terrain.radius * (0.11 + 0.06 * candidate.intensity))
      }
    })

    // 选中环脉动
    if (this.selectedId && this.selectionMaterial.opacity > 0) {
      this.selectionMaterial.opacity = 0.55 + 0.3 * Math.sin(now / 320)
    }
  }

  private applyTraffic(frame: IslandFrame, dt: number): void {
    for (const traffic of frame.traffic) {
      const link = this.links.get(traffic.linkId)
      if (!link) continue
      link.glowMaterial.emissiveIntensity = traffic.active ? 0.8 : 0.05
      link.glowMaterial.opacity = traffic.active ? 0.55 : 0.16
      link.cargo.visible = traffic.active
      if (traffic.active && traffic.phase >= 0) {
        const point = link.curve.getPointAt(Math.min(0.999, Math.max(0.001, traffic.phase)))
        link.cargo.position.copy(point)
        link.cargo.rotation.y += dt * 3
      }
    }
  }

  private updateWater(): void {
    // 岸线浪花随时间明灭（潮汐感）；reduce-motion 下保持恒定
    this.water.foamMaterial.opacity = this.reducedMotion
      ? 0.3
      : 0.24 + 0.12 * Math.sin(this.elapsed * this.spec.water.waveSpeed * 0.9)
    if (this.reducedMotion) return
    const position = this.waterPosition
    const base = this.waterBase
    for (let i = 0; i < position.count; i++) {
      const x = base[i * 3]
      const z = base[i * 3 + 2]
      position.setY(i, waterWaveAt(this.spec, x, z, this.elapsed))
    }
    position.needsUpdate = true
  }

  private updateClouds(dt: number): void {
    if (this.reducedMotion) return
    const drift = this.spec.motion.cloudSpeed * dt
    this.clouds.rotation.y += drift * 0.02
    for (const cloud of this.clouds.children) {
      cloud.position.x += drift * 0.12
      const limit = this.spec.water.radius
      if (cloud.position.x > limit) cloud.position.x = -limit
    }
  }

  private updateCamera(dt: number): void {
    this.cameraState = this.reducedMotion
      ? { ...this.cameraGoal }
      : dampCamera(this.cameraState, this.cameraGoal, dt, 8)
    const position = cameraPositionOf(this.cameraState)
    this.camera.position.set(position.x, position.y, position.z)
    this.camera.lookAt(this.cameraState.targetX, this.cameraState.targetY, this.cameraState.targetZ)
  }

  private emitLabels(now: number): void {
    if (!this.onLabels || now - this.lastLabels < 100) return
    this.lastLabels = now
    const width = this.container.clientWidth
    const height = this.container.clientHeight
    const anchors: IslandLabelAnchor[] = []
    const boxes = new Map<string, LabelBox>()
    for (const [id, handle] of this.entities) {
      const point = new THREE.Vector3(handle.entity.plot.x, handle.topY, handle.entity.plot.z).project(this.camera)
      const px = (point.x * 0.5 + 0.5) * width
      const py = (-point.y * 0.5 + 0.5) * height
      const box = estimateLabelBox(this.labelTitle?.(handle.entity) ?? handle.entity.title)
      boxes.set(id, box)
      anchors.push({
        id,
        x: px,
        y: py,
        // 只在标签能整块落在画布内时显示：贴着画布上沿的锚点会把胶囊切掉一半，
        // 而画布外（聚焦后建筑顶部已在视口上方）的锚点会变成不可见的幽灵按钮。
        visible: point.z < 1 && anchorFitsCanvas(px, py, width, height, box),
        selected: id === this.selectedId,
      })
    }
    this.onLabels(separateLabelAnchors(anchors, boxes, width, height))
  }

  private emptySky(): IslandSkyState {
    return {
      phase: 'day',
      dayProgress: 0.3,
      dayFactor: 1,
      nightFactor: 0,
      sunAngle: 0,
      sunDirection: { x: 0.6, y: 0.7, z: 0.4 },
      sunColor: this.spec.lighting.dayColor,
      sunIntensity: 2.2,
      ambientColor: this.spec.palette.cloud,
      ambientIntensity: this.spec.lighting.ambientDay,
      skyTop: mixHex(this.spec.palette.water, this.spec.lighting.dayColor, 0.5),
      skyBottom: mixHex(this.spec.palette.cloud, this.spec.lighting.dayColor, 0.35),
      fogColor: mixHex(this.spec.palette.cloud, this.spec.lighting.dayColor, 0.35),
      beamIntensity: 0.2,
    }
  }

  /* ── 交互 ── */

  private attachPointerHandlers(): void {
    const el = this.renderer.domElement
    el.addEventListener('pointerdown', this.handlePointerDown)
    el.addEventListener('pointermove', this.handlePointerMove)
    el.addEventListener('pointerup', this.handlePointerUp)
    el.addEventListener('pointercancel', this.handlePointerUp)
    el.addEventListener('pointerleave', this.handlePointerLeave)
    el.addEventListener('wheel', this.handleWheel, { passive: false })
    el.addEventListener('dblclick', this.handleDoubleClick)
    el.addEventListener('contextmenu', this.handleContextMenu)
  }

  private detachPointerHandlers(): void {
    const el = this.renderer.domElement
    el.removeEventListener('pointerdown', this.handlePointerDown)
    el.removeEventListener('pointermove', this.handlePointerMove)
    el.removeEventListener('pointerup', this.handlePointerUp)
    el.removeEventListener('pointercancel', this.handlePointerUp)
    el.removeEventListener('pointerleave', this.handlePointerLeave)
    el.removeEventListener('wheel', this.handleWheel)
    el.removeEventListener('dblclick', this.handleDoubleClick)
    el.removeEventListener('contextmenu', this.handleContextMenu)
  }

  private handleContextMenu = (event: MouseEvent): void => {
    event.preventDefault()
  }

  private handlePointerDown = (event: PointerEvent): void => {
    this.renderer.domElement.setPointerCapture?.(event.pointerId)
    this.pointerStart = { x: event.clientX, y: event.clientY, time: performance.now() }
    this.pointerLast = { x: event.clientX, y: event.clientY }
    this.pointerMode = event.button === 2 || event.button === 1 || event.shiftKey ? 'pan' : 'orbit'
    this.renderer.domElement.style.cursor = 'grabbing'
  }

  private handlePointerMove = (event: PointerEvent): void => {
    const height = Math.max(1, this.container.clientHeight)
    if (this.pointerMode === 'none') {
      const now = performance.now()
      if (now - this.lastHoverCheck > 90) {
        this.lastHoverCheck = now
        this.hover(this.pickEntityAt(event))
      }
      return
    }
    const dx = event.clientX - this.pointerLast.x
    const dy = event.clientY - this.pointerLast.y
    this.pointerLast = { x: event.clientX, y: event.clientY }
    if (this.pointerMode === 'orbit') {
      this.cameraGoal = orbitCamera(this.cameraGoal, dx / height * 3.2, dy / height * 2.6, this.spec)
    } else {
      const scale = this.cameraGoal.distance / height
      this.cameraGoal = panCamera(this.cameraGoal, -dx * scale, dy * scale, this.spec)
    }
  }

  private handlePointerUp = (event: PointerEvent): void => {
    const movement = Math.hypot(event.clientX - this.pointerStart.x, event.clientY - this.pointerStart.y)
    const elapsed = performance.now() - this.pointerStart.time
    const wasClick = this.pointerMode !== 'none' && movement < 6 && elapsed < 420 && event.button === 0
    this.pointerMode = 'none'
    this.renderer.domElement.releasePointerCapture?.(event.pointerId)
    this.renderer.domElement.style.cursor = this.hoveredId ? 'pointer' : 'grab'
    if (wasClick) {
      const id = this.pickEntityAt(event)
      this.select(id)
      this.onSelect?.(id)
    }
  }

  private handlePointerLeave = (): void => {
    this.hover(null)
    this.pointerMode = 'none'
  }

  private handleWheel = (event: WheelEvent): void => {
    event.preventDefault()
    this.cameraGoal = zoomCamera(this.cameraGoal, Math.sign(event.deltaY) * 0.16, this.spec)
  }

  private handleDoubleClick = (event: MouseEvent): void => {
    const id = this.pickEntityAt(event)
    if (id) this.focusEntity(id)
  }

  private pickEntityAt(event: MouseEvent): string | null {
    const rect = this.renderer.domElement.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return null
    this.pointerNdc.set(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1,
    )
    this.raycaster.setFromCamera(this.pointerNdc, this.camera)
    const hits = this.raycaster.intersectObjects(this.entityGroup.children, true)
    for (const hit of hits) {
      let current: THREE.Object3D | null = hit.object
      while (current) {
        const id = current.userData?.entityId
        if (typeof id === 'string') return id
        current = current.parent
      }
    }
    return null
  }

  private handleContextLost = (event: Event): void => {
    event.preventDefault()
    this.paused = true
    this.onContextLost?.()
  }

  /* ── 观察器 ── */

  private setupObservers(): void {
    this.resizeObserver = new ResizeObserver(() => this.handleResize())
    this.resizeObserver.observe(this.container)

    if (typeof IntersectionObserver === 'function') {
      this.intersectionObserver = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) this.setPaused(!entry.isIntersecting)
        },
        { threshold: 0.02 },
      )
      this.intersectionObserver.observe(this.container)
    }

    if (typeof document !== 'undefined') {
      this.onVisibilityChange = () => this.setPaused(document.hidden)
      document.addEventListener('visibilitychange', this.onVisibilityChange)
    }

    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
      this.motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
      this.reducedMotion = this.motionQuery.matches
      this.onMotionChange = (event) => {
        this.reducedMotion = event.matches
      }
      this.motionQuery.addEventListener('change', this.onMotionChange)
    }
  }

  private handleResize(): void {
    const width = Math.max(1, this.container.clientWidth)
    const height = Math.max(1, this.container.clientHeight)
    const aspect = width / height
    this.camera.aspect = aspect
    this.applyCameraFraming(aspect)
    this.renderer.setSize(width, height)
  }

  /**
   * 窄窗格补偿：宽高比 < 1 时按宽度换算纵向视角，
   * 保证岛屿横向完整入画（工作台侧栏里的岛屿不该被裁掉两边）。
   */
  private applyCameraFraming(aspect: number): void {
    this.camera.fov = aspect >= 1
      ? this.baseFov
      : THREE.MathUtils.radToDeg(2 * Math.atan(Math.tan(THREE.MathUtils.degToRad(this.baseFov / 2)) / aspect))
    this.camera.updateProjectionMatrix()
  }
}

function entityGroundY(entity: IslandEntity): number {
  return entity.kind === 'pier' || ISLAND_KIND_META[entity.kind].waterborne ? 0.3 : 0.05
}

function cameraPositionOf(cam: OrbitCamera): { x: number; y: number; z: number } {
  const horizontal = cam.distance * Math.cos(cam.pitch)
  return {
    x: cam.targetX + horizontal * Math.sin(cam.yaw),
    y: cam.targetY + cam.distance * Math.sin(cam.pitch),
    z: cam.targetZ + horizontal * Math.cos(cam.yaw),
  }
}

function mixHex(a: number, b: number, t: number): number {
  const k = t < 0 ? 0 : t > 1 ? 1 : t
  const r = Math.round(((a >> 16) & 0xff) + (((b >> 16) & 0xff) - ((a >> 16) & 0xff)) * k)
  const g = Math.round(((a >> 8) & 0xff) + (((b >> 8) & 0xff) - ((a >> 8) & 0xff)) * k)
  const bl = Math.round((a & 0xff) + ((b & 0xff) - (a & 0xff)) * k)
  return (r << 16) | (g << 8) | bl
}