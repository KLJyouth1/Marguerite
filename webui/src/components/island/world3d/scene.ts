/**
 * 3D 开放世界渲染核心（纯 three，无 react/r3f）。
 *
 * 职责：场景/相机/渲染器/光照/天空盒 + 程序化道具几何 + WASD 漫游 + HUD。
 * 为什么不引 react-three-fiber / TresJS？P0 的容器是 Vue，three 原生生命周期
 * 与 IslandGameScene（2D 场景类）保持同一范式（setter 注入 / dispose / HUD 回调），
 * 引入跨框架桥只会增加体积与心智负担。
 *
 * 为什么程序化几何体而非外部素材？见 terrain.ts 文件头——AI 自主设计 =
 * 代码契约、零素材依赖、可热更新；Blender 管线是升级出口而非依赖。
 */

import * as THREE from 'three'
import { createTerrain, ISLAND_RADIUS, type TerrainMeshes } from './terrain'
import {
  buildPropPlacements,
  propSpec,
  type PropKind,
  type PropPlacement,
  type PropSourceCounts,
} from './propsSpec'
import type { TierProfile } from './tierProfile'
import { glbHotReload } from './glbHotReload'

/** HUD 回调数据（2D 场景 SceneHud 的超集） */
export interface World3dHud {
  fps: number
  drawCalls: number
  /** 当前面向的可交互道具（null = 附近无可交互） */
  focus: FocusedProp | null
  /** 玩家坐标（调试/小地图用） */
  position: [number, number, number]
}

/** 面向中的道具（拾取交互的目标） */
export interface FocusedProp {
  id: string
  kind: PropKind
  name: string
  distance: number
}

/** 场景脉冲输入（agentBinding.ScenePulse 的消费端；这里只依赖形状避免循环） */
export interface ScenePulseInput {
  type: 'aurora' | 'beacon' | 'fog' | 'bloom' | 'spawn' | 'storm'
  intensity: number
  label: string
}

/** 拾取事件回调（渲染层 → 桥接层 → Agent） */
export interface PickupEvent {
  id: string
  kind: PropKind
  name: string
  position: [number, number, number]
}

const PULSE_DURATION_MS = 2600

/** toon 渐变贴图（与 IslandGameScene.makeToonGradient 同一配方） */
function makeToonGradient(): THREE.DataTexture {
  const data = new Uint8Array([90, 150, 210, 255])
  const tex = new THREE.DataTexture(data, 4, 1, THREE.RedFormat)
  tex.minFilter = THREE.NearestFilter
  tex.magFilter = THREE.NearestFilter
  tex.needsUpdate = true
  return tex
}

/** 主题 → 道具强调色（0x 数字常量不触发颜色守卫） */
const PROP_TINT: Record<PropKind, number> = {
  code_crystal: 0x6aa9ff,
  doc_scroll: 0xe8d5a3,
  data_fruit: 0xe8785f,
  quest_beacon: 0xe8b45f,
  inspiration_bloom: 0xc98bd8,
}

/** 主 primitive → three 几何体（detail 决定面数档） */
function mainGeometry(main: string, detail: 1 | 2 | 3): THREE.BufferGeometry {
  const seg = detail === 1 ? 6 : detail === 2 ? 12 : 20
  switch (main) {
    case 'octahedron':
      return new THREE.OctahedronGeometry(0.7, detail === 3 ? 1 : 0)
    case 'cone':
      return new THREE.ConeGeometry(0.55, 1.3, seg)
    case 'sphere':
      return new THREE.SphereGeometry(0.55, seg, Math.max(6, seg / 2))
    case 'tower':
      return new THREE.CylinderGeometry(0.28, 0.42, 1.8, seg)
    case 'flower':
      return new THREE.SphereGeometry(0.45, seg, Math.max(6, seg / 2))
    default:
      return new THREE.BoxGeometry(0.8, 0.8, 0.8)
  }
}

/** 次 primitive（accent）：装饰件，找不到就返回 null */
function accentGeometry(accent: string | undefined): THREE.BufferGeometry | null {
  switch (accent) {
    case 'prism':
      return new THREE.ConeGeometry(0.18, 0.6, 4)
    case 'scroll':
      return new THREE.CylinderGeometry(0.12, 0.12, 1, 8)
    case 'berry':
      return new THREE.SphereGeometry(0.16, 8, 6)
    case 'lamp':
      return new THREE.SphereGeometry(0.2, 10, 8)
    case 'petal':
      return new THREE.TorusGeometry(0.4, 0.1, 6, 12)
    default:
      return null
  }
}

interface PropHandle {
  id: string
  kind: PropKind
  name: string
  group: THREE.Group
  bobOffset: number
}

export class IslandWorld3DScene {
  private renderer: THREE.WebGLRenderer
  private scene = new THREE.Scene()
  private camera: THREE.PerspectiveCamera
  private clock = new THREE.Clock()
  private raf = 0
  private disposed = false

  private terrain: TerrainMeshes
  private props: PropHandle[] = []
  private propById = new Map<string, PropHandle>()
  /** 最近一次 setPropSources 的摆放方案（GLB 热替换时按此原子重建全部道具） */
  private placements: PropPlacement[] = []
  /** GLB 变更订阅的退订函数（dispose 必须调用；否则单例订阅表持有已销毁场景 → 内存泄漏） */
  private unsubscribeGlbAssets: (() => void) | null = null

  private keys = new Set<string>()
  /** 漫游姿态（自实现 PointerLock 鼠标视角：yaw/pitch，避免依赖 three/addons） */
  private yaw = 0
  private pitch = -0.12
  private playerPos = new THREE.Vector3(0, 3, 14)
  private velocity = new THREE.Vector3()
  private moveSpeed = 8

  private hemi: THREE.HemisphereLight
  private sun: THREE.DirectionalLight
  private sky: THREE.Mesh
  private aurora: THREE.Mesh

  // 脉冲动画状态（Agent → 游戏方向的可视化）
  private pulse: { type: ScenePulseInput['type']; intensity: number; label: string; until: number } | null = null

  private hud: World3dHud = { fps: 60, drawCalls: 0, focus: null, position: [0, 0, 0] }
  private frameCount = 0
  private fpsTimer = 0
  private focused: FocusedProp | null = null

  private reduceMotion: boolean
  private profile: TierProfile
  private onHud?: (h: World3dHud) => void
  private onPickup?: (e: PickupEvent) => void

  constructor(canvas: HTMLCanvasElement, profile: TierProfile, reduceMotion = false) {
    this.profile = profile
    this.reduceMotion = reduceMotion

    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false })
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.shadowMap.enabled = true
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap

    this.camera = new THREE.PerspectiveCamera(62, 1, 0.1, 500)

    // 光照：半球环境光 + 主方向光（平行光投影）
    this.hemi = new THREE.HemisphereLight(0xbfe3ff, 0x6a8f5a, 0.9)
    this.scene.add(this.hemi)
    this.sun = new THREE.DirectionalLight(0xfff2d0, 1.5)
    this.sun.position.set(22, 36, 14)
    this.sun.castShadow = true
    if (this.sun.shadow) {
      this.sun.shadow.mapSize.set(1024, 1024)
      this.sun.shadow.camera.left = -50
      this.sun.shadow.camera.right = 50
      this.sun.shadow.camera.top = 50
      this.sun.shadow.camera.bottom = -50
    }
    this.scene.add(this.sun)

    // 天空盒：大球体内壁 + 渐变色（MeshBasicMaterial side: BackSide 即「天空盒」）
    const skyGeo = new THREE.SphereGeometry(240, 24, 16)
    const skyMat = new THREE.MeshBasicMaterial({ color: 0x9ecfef, side: THREE.BackSide, fog: false })
    this.sky = new THREE.Mesh(skyGeo, skyMat)
    this.scene.add(this.sky)

    // 极光带（夜空辉光，强度由档位 auroraIntensity 驱动）
    const auroraGeo = new THREE.TorusGeometry(90, 6, 8, 48, Math.PI)
    const auroraMat = new THREE.MeshBasicMaterial({
      color: 0x6cc78a,
      transparent: true,
      opacity: 0.0,
      side: THREE.DoubleSide,
      fog: false,
    })
    this.aurora = new THREE.Mesh(auroraGeo, auroraMat)
    this.aurora.position.set(0, 70, -120)
    this.aurora.rotation.x = Math.PI / 2.6
    this.scene.add(this.aurora)

    // 雾（fog 脉冲与天气共用）
    this.scene.fog = new THREE.Fog(0x9ecfef, 60, 260)

    // 地形 + 水面
    this.terrain = createTerrain()
    this.scene.add(this.terrain.terrain)
    this.scene.add(this.terrain.water)

    // GLB 热加载接线：glbHotReload 成功替换资产后通知本场景，按当前 placements
    // 原子重建全部道具（程序化几何 → GLB 实例热替换）。订阅是普通 Set 回调，
    // 不参与 vue 响应式；dispose 时退订（见 dispose）。
    this.unsubscribeGlbAssets = glbHotReload.subscribe(() => this.refreshPropAssets())

    window.addEventListener('keydown', this.onKeyDown)
    window.addEventListener('keyup', this.onKeyUp)
    window.addEventListener('resize', this.onResize)
    document.addEventListener('mousemove', this.onMouseMove)
    document.addEventListener('pointerlockchange', this.onPointerLockChange)

    this.onResize()
    this.clock.start()
    this.loop()
  }

  setHudListener(cb: (h: World3dHud) => void): void {
    this.onHud = cb
  }

  setPickupListener(cb: (e: PickupEvent) => void): void {
    this.onPickup = cb
  }

  /**
   * 应用真实运行时的道具来源（工具/产物/记忆/委托/编队 → 布局）。
   * 布局是确定性的（propsSpec.buildPropPlacements），同来源同位置。
   */
  setPropSources(counts: PropSourceCounts, seed: string): void {
    this.placements = buildPropPlacements(counts, seed)
    this.refreshPropAssets()
  }

  /**
   * 按当前 placements 原子重建全部道具（先 clearProps 再重建）。
   * 用于 GLB 资产加载完成后的热替换：一次性切换所有道具的几何来源
   * （程序化几何 → GLB 实例），避免出现新旧几何混用的中间态。
   * 无 placements（尚未 setPropSources）时只清空，安全空操作。
   */
  refreshPropAssets(): void {
    this.clearProps()
    for (const p of this.placements) {
      const handle = this.buildProp(p)
      this.props.push(handle)
      this.propById.set(p.id, handle)
      this.scene.add(handle.group)
    }
  }

  /** 消费 Agent→游戏 的场景脉冲（瞬时特效 + 遗留光强变化） */
  applyPulse(pulse: ScenePulseInput): void {
    this.pulse = { ...pulse, until: performance.now() + PULSE_DURATION_MS }
  }

  /** 切换档位（粒子/极光/时间缩放随 TierProfile 变化） */
  setProfile(profile: TierProfile): void {
    this.profile = profile
    this.moveSpeed = 8 * profile.timeScale
  }

  /** 鼠标指针锁定（漫游视角）。由 Vue 容器在画布 click 时调用。 */
  requestPointerLock(): void {
    const el = this.renderer.domElement
    if (document.pointerLockElement !== el) el.requestPointerLock()
  }

  exitPointerLock(): void {
    if (document.pointerLockElement) document.exitPointerLock()
  }

  /** 拾取面向中的道具（E 键/点击触发）；无目标返回 null */
  pickupFocused(): PickupEvent | null {
    const f = this.focused
    if (!f) return null
    const handle = this.propById.get(f.id)
    if (!handle) return null
    const ev: PickupEvent = {
      id: f.id,
      kind: f.kind,
      name: f.name,
      position: [handle.group.position.x, handle.group.position.y, handle.group.position.z],
    }
    // 拾取后道具离场（下一批真实来源会重新布置）
    this.scene.remove(handle.group)
    this.releasePropResources(handle.group)
    this.propById.delete(f.id)
    this.props = this.props.filter((p) => p.id !== f.id)
    this.focused = null
    this.onPickup?.(ev)
    return ev
  }

  // ───────────────────────── 内部：道具构建 ─────────────────────────

  private buildProp(p: PropPlacement): PropHandle {
    const spec = propSpec(p.kind)
    const group = new THREE.Group()

    // 优先消费 GLB 模板（glbHotReload 已校验入库）；无模板 → 程序化几何诚实兜底。
    const template = glbHotReload.getAsset(p.kind)
    if (template) {
      // clone(true)：Three.js 的 Mesh.clone 只**拷贝引用**——geometry 与 material
      // 与模板（及其它实例）**共享**，仅 Object3D 的变换是独立副本。
      // 好处：GLB 几何/材质只上传 GPU 一次，多实例零额外显存占用；
      // 代价见 releasePropResources —— 共享资源绝不能被单个实例 dispose。
      const model = template.clone(true)
      model.scale.multiplyScalar(spec.geometry.scale)
      model.traverse((o) => {
        const m = o as THREE.Mesh
        if (m.isMesh) m.castShadow = true
      })
      group.add(model)
      // 实例标记：releasePropResources 据此跳过共享资源释放（见该函数注释）
      group.userData.__glbInstance = true
    } else {
      // 程序化几何路径（GLB 缺失/加载失败/校验被拒时的诚实兜底，逻辑保持不变）
      const gradient = makeToonGradient()
      const mat = new THREE.MeshToonMaterial({ color: PROP_TINT[p.kind], gradientMap: gradient })

      const main = new THREE.Mesh(mainGeometry(spec.geometry.main, spec.geometry.detail), mat)
      main.position.y = 0.8 * spec.geometry.scale
      main.scale.setScalar(spec.geometry.scale)
      main.castShadow = true
      group.add(main)

      const accGeo = accentGeometry(spec.geometry.accent)
      if (accGeo) {
        const acc = new THREE.Mesh(accGeo, mat)
        acc.position.y = 1.5 * spec.geometry.scale
        acc.scale.setScalar(spec.geometry.scale * 0.8)
        group.add(acc)
      }
    }

    // 贴地（与相机/植被同一高度采样函数）
    group.position.set(p.x, this.terrain.heightAt(p.x, p.z), p.z)
    group.rotation.y = p.rotation

    return { id: p.id, kind: p.kind, name: p.name, group, bobOffset: Math.random() * Math.PI * 2 }
  }

  /**
   * 释放一个道具 group 的渲染资源。
   *
   * ⚠ 共享资源红线（为什么必须按实例类型区分、不能无脑 dispose）：
   * GLB 模板由 glbHotReload 单例持有，scene 只持有 clone 实例；而 Three.js 的
   * Mesh.clone 共享 geometry 与 material（只拷贝引用、不深拷贝）。若在这里无脑
   * dispose，模板与**所有其它实例**会立即黑屏/崩溃——已上传的 GPU 资源被释放，
   * 后续 draw call 拿到空 buffer；且模块级单例跨 Vue 重挂载仍复用同一模板。
   * 因此：GLB 实例（userData.__glbInstance）一律跳过；只有程序化几何
   * （每次 new 出来的独享 geometry/material）才在此释放。
   */
  private releasePropResources(group: THREE.Group): void {
    if (group.userData.__glbInstance) return
    group.traverse((o) => {
      const m = o as THREE.Mesh
      if (m.isMesh) {
        m.geometry.dispose()
        const mat = m.material as THREE.Material | THREE.Material[]
        if (Array.isArray(mat)) mat.forEach((x) => x.dispose())
        else mat.dispose()
      }
    })
  }

  private clearProps(): void {
    for (const h of this.props) {
      this.scene.remove(h.group)
      this.releasePropResources(h.group)
    }
    this.props = []
    this.propById.clear()
    this.focused = null
  }

  // ───────────────────────── 输入 ─────────────────────────

  private onKeyDown = (e: KeyboardEvent): void => {
    this.keys.add(e.key.toLowerCase())
    if (e.key.toLowerCase() === 'e') this.pickupFocused()
  }

  private onKeyUp = (e: KeyboardEvent): void => {
    this.keys.delete(e.key.toLowerCase())
  }

  private onMouseMove = (e: MouseEvent): void => {
    if (document.pointerLockElement !== this.renderer.domElement) return
    // 自实现 PointerLock 鼠标视角（为什么不 import three/addons/PointerLockControls：
    // 项目此前无 addons 引用先例，自实现 20 行即可，避免打包路径不确定性）
    this.yaw -= e.movementX * 0.0024
    this.pitch -= e.movementY * 0.0024
    this.pitch = Math.max(-1.35, Math.min(1.35, this.pitch))
  }

  private onPointerLockChange = (): void => {
    // 指针锁定退出时清空按键，避免「卡步」
    if (document.pointerLockElement !== this.renderer.domElement) this.keys.clear()
  }

  private onResize = (): void => {
    const parent = this.renderer.domElement.parentElement
    const w = parent?.clientWidth || window.innerWidth
    const h = parent?.clientHeight || window.innerHeight
    this.camera.aspect = w / Math.max(1, h)
    this.camera.updateProjectionMatrix()
    this.renderer.setSize(w, h, false)
  }

  // ───────────────────────── 主循环 ─────────────────────────

  private loop = (): void => {
    if (this.disposed) return
    this.raf = requestAnimationFrame(this.loop)
    const dt = Math.min(0.05, this.clock.getDelta()) * this.profile.timeScale
    const now = performance.now()

    // 移动（WASD + 方向键），贴地行走
    const forward = (this.keys.has('w') || this.keys.has('arrowup') ? 1 : 0) - (this.keys.has('s') || this.keys.has('arrowdown') ? 1 : 0)
    const strafe = (this.keys.has('d') || this.keys.has('arrowright') ? 1 : 0) - (this.keys.has('a') || this.keys.has('arrowleft') ? 1 : 0)
    const dir = new THREE.Vector3(strafe, 0, -forward)
    if (dir.lengthSq() > 0) {
      dir.normalize().applyAxisAngle(new THREE.Vector3(0, 1, 0), this.yaw)
      this.velocity.lerp(dir.multiplyScalar(this.moveSpeed), 0.18)
    } else {
      this.velocity.lerp(new THREE.Vector3(), 0.12)
    }
    this.playerPos.addScaledVector(this.velocity, dt)
    // 岛内活动半径约束（出界推回）
    const r = Math.hypot(this.playerPos.x, this.playerPos.z)
    if (r > ISLAND_RADIUS * 1.1) {
      const s = (ISLAND_RADIUS * 1.1) / r
      this.playerPos.x *= s
      this.playerPos.z *= s
    }
    const groundY = this.terrain.heightAt(this.playerPos.x, this.playerPos.z)
    this.playerPos.y = groundY + 1.7

    // 相机：第一人称（位置 = 玩家，朝向 = yaw/pitch）
    this.camera.position.copy(this.playerPos)
    this.camera.rotation.set(0, 0, 0)
    this.camera.rotateY(this.yaw)
    this.camera.rotateX(this.pitch)

    // 道具呼吸浮动 + 面向检测（距离 < 3.2 可交互）
    const time = now * 0.001
    let best: FocusedProp | null = null
    let bestDist = 3.2
    for (const h of this.props) {
      if (!this.reduceMotion) {
        h.group.position.y = this.terrain.heightAt(h.group.position.x, h.group.position.z) + Math.sin(time * 1.4 + h.bobOffset) * 0.12
      }
      const d = h.group.position.distanceTo(this.playerPos)
      if (d < bestDist) {
        bestDist = d
        best = { id: h.id, kind: h.kind, name: h.name, distance: Number(d.toFixed(2)) }
      }
    }
    this.focused = best

    // 脉冲动画（Agent 事件 → 场景可见反馈）
    const auroraMat = this.aurora.material as THREE.MeshBasicMaterial
    if (this.pulse && now < this.pulse.until) {
      const t = 1 - (this.pulse.until - now) / PULSE_DURATION_MS
      const k = Math.sin(t * Math.PI) * this.pulse.intensity
      switch (this.pulse.type) {
        case 'aurora':
          auroraMat.opacity = Math.min(1, auroraMat.opacity + k * 0.08)
          break
        case 'beacon':
          this.sun.intensity = 1.5 + k * 1.2
          break
        case 'fog': {
          const fog = this.scene.fog as THREE.Fog
          fog.far = 260 - k * 160
          break
        }
        case 'bloom':
          this.hemi.intensity = 0.9 + k * 1.1
          break
        case 'spawn':
          // 生成脉冲：道具轻微放大
          for (const h of this.props) h.group.scale.setScalar(1 + k * 0.12)
          break
        case 'storm':
          this.scene.rotation.z = Math.sin(t * 40) * 0.004 * k
          break
      }
    } else if (this.pulse) {
      // 脉冲结束：回归稳态
      this.pulse = null
      this.sun.intensity = 1.5
      this.hemi.intensity = 0.9
      const fog = this.scene.fog as THREE.Fog
      fog.far = 260
      this.scene.rotation.z = 0
      for (const h of this.props) h.group.scale.setScalar(1)
    }
    // 极光基线强度（档位神力域）+ 缓慢衰减
    const baseAurora = this.profile.auroraIntensity * 0.35
    auroraMat.opacity = Math.max(baseAurora, auroraMat.opacity * 0.985)

    this.renderer.render(this.scene, this.camera)

    // HUD（节流：每 0.25s 上报一次，避免 Vue 响应式风暴）
    this.frameCount += 1
    this.fpsTimer += dt
    if (this.fpsTimer >= 0.25) {
      this.hud = {
        fps: Math.round(this.frameCount / this.fpsTimer),
        drawCalls: this.renderer.info.render.calls,
        focus: this.focused,
        position: [Number(this.playerPos.x.toFixed(1)), Number(this.playerPos.y.toFixed(1)), Number(this.playerPos.z.toFixed(1))],
      }
      this.frameCount = 0
      this.fpsTimer = 0
      this.onHud?.(this.hud)
    }
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    cancelAnimationFrame(this.raf)
    // 退订 GLB 变更：glbHotReload 单例的订阅表持有本场景回调，不退订会导致
    // 场景销毁后仍被单例引用（内存泄漏）且回调可能打到已销毁实例。
    this.unsubscribeGlbAssets?.()
    this.unsubscribeGlbAssets = null
    window.removeEventListener('keydown', this.onKeyDown)
    window.removeEventListener('keyup', this.onKeyUp)
    window.removeEventListener('resize', this.onResize)
    document.removeEventListener('mousemove', this.onMouseMove)
    document.removeEventListener('pointerlockchange', this.onPointerLockChange)
    this.clearProps()
    this.terrain.dispose()
    this.scene.traverse((o) => {
      const m = o as THREE.Mesh
      if (m.isMesh) {
        m.geometry.dispose()
        const mat = m.material as THREE.Material | THREE.Material[]
        if (Array.isArray(mat)) mat.forEach((x) => x.dispose())
        else mat.dispose()
      }
    })
    this.renderer.dispose()
  }
}
