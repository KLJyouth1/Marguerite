/**
 * 3D 开放世界岛场景（MVP）—— 程序化低多边形三渲二。
 *
 * 复用 P3/Island3D 已验证范式：
 * - WebGLRenderer + outputColorSpace SRGB + PCFSoftShadow（仅 high 档）；
 * - MeshToonMaterial + 4 阶渐变贴图（cel shading）；
 * - 主题色读自 CSS 语义 token（readIslandThemeTokens），不硬编码品牌色。
 *
 * 性能预算（蓝图 §6）：
 * - 几何体/材质全部复用（每类一份），花草用 InstancedMesh；
 * - high 档开阴影，medium/low 关阴影并减装饰物；
 * - rAF 主循环 + dispose() 完整释放（无 GPU 泄漏）。
 *
 * 本类不做业务判断：天气/昼夜/编队数量由 Vue 层通过 setter 推入。
 */

import * as THREE from 'three'
import { readIslandThemeTokens } from '../Island3D'
import type { WorldSpec, WeatherKind, WorldTheme } from './worldSpec'

export type IslandGameTier = 'high' | 'medium' | 'low'

export interface SceneHud {
  fps: number
  drawCalls: number
}

interface LandmarkHandle {
  group: THREE.Group
  /** 模式符文灯（bloom 时点亮层数） */
  runeMaterials: THREE.MeshToonMaterial[]
}

function makeToonGradient(): THREE.DataTexture {
  const data = new Uint8Array([90, 150, 210, 255])
  const tex = new THREE.DataTexture(data, 4, 1, THREE.RedFormat)
  tex.minFilter = THREE.NearestFilter
  tex.magFilter = THREE.NearestFilter
  tex.needsUpdate = true
  return tex
}

const THEME_COLOR: Record<WorldTheme, number> = {
  verdant: 0x7ec850,
  blossom: 0xf4a7c3,
  twilight: 0x9a8bd4,
}

export class IslandGameScene {
  private renderer: THREE.WebGLRenderer
  private scene = new THREE.Scene()
  private camera: THREE.PerspectiveCamera
  private clock = new THREE.Clock()
  private raf = 0
  private disposed = false

  private tier: IslandGameTier
  private keys = new Set<string>()
  private player = new THREE.Group()
  private pet = new THREE.Group()
  private elves: THREE.Group[] = []
  private water!: THREE.Mesh
  private rain?: THREE.Points
  private fireflies?: THREE.Points
  private fireflyBase?: Float32Array
  private fireflyCount = -1
  private landmark?: LandmarkHandle
  private treeMats: THREE.MeshToonMaterial[] = []

  private hemi: THREE.HemisphereLight
  private sun: THREE.DirectionalLight

  private hud: SceneHud = { fps: 60, drawCalls: 0 }
  private frameCount = 0
  private fpsTimer = 0

  private reduceMotion: boolean
  private onHud?: (h: SceneHud) => void

  constructor(canvas: HTMLCanvasElement, tier: IslandGameTier, reduceMotion = false) {
    this.tier = tier
    this.reduceMotion = reduceMotion
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: tier !== 'low', alpha: false })
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.shadowMap.enabled = tier === 'high'
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap

    this.camera = new THREE.PerspectiveCamera(58, 1, 0.1, 400)
    this.camera.position.set(0, 9, 16)

    const tokens = readIslandThemeTokens()
    this.hemi = new THREE.HemisphereLight(0xbfe3ff, 0x6a8f5a, 0.9)
    this.scene.add(this.hemi)
    this.sun = new THREE.DirectionalLight(0xfff2d0, 1.4)
    this.sun.position.set(18, 30, 12)
    this.sun.castShadow = tier === 'high'
    if (this.sun.castShadow && this.sun.shadow) {
      this.sun.shadow.mapSize.set(1024, 1024)
      this.sun.shadow.camera.left = -40
      this.sun.shadow.camera.right = 40
      this.sun.shadow.camera.top = 40
      this.sun.shadow.camera.bottom = -40
    }
    this.scene.add(this.sun)

    const gradient = makeToonGradient()
    const groundMat = new THREE.MeshToonMaterial({ color: THEME_COLOR.verdant, gradientMap: gradient })
    this.treeMats.push(groundMat)

    // 岛体（扁圆柱）
    const island = new THREE.Mesh(new THREE.CylinderGeometry(40, 46, 7, 28), groundMat)
    island.position.y = -3.5
    island.receiveShadow = true
    this.scene.add(island)

    // 海面
    this.water = new THREE.Mesh(
      new THREE.CircleGeometry(160, 40),
      new THREE.MeshBasicMaterial({ color: 0x5aa9d6, transparent: true, opacity: 0.55 }),
    )
    this.water.rotation.x = -Math.PI / 2
    this.water.position.y = -3.2
    this.scene.add(this.water)

    this.buildLandmarks(gradient, tokens.accent)
    this.buildActors(gradient, tokens)

    window.addEventListener('keydown', this.onKeyDown)
    window.addEventListener('keyup', this.onKeyUp)
    window.addEventListener('resize', this.onResize)

    this.onResize()
    this.clock.start()
    this.loop()
  }

  setHudListener(cb: (h: SceneHud) => void): void {
    this.onHud = cb
  }

  private buildLandmarks(gradient: THREE.DataTexture, accent: number): void {
    const group = new THREE.Group()
    const runeMaterials: THREE.MeshToonMaterial[] = []

    // 中枢庭 · 雏菊圣殿（十瓣花顶圆厅）
    const stone = new THREE.MeshToonMaterial({ color: 0xf3ead8, gradientMap: gradient })
    const base = new THREE.Mesh(new THREE.CylinderGeometry(5, 6, 2.5, 12), stone)
    base.position.y = 1.25
    base.castShadow = true
    group.add(base)
    // 十瓣花瓣
    const petalMat = new THREE.MeshToonMaterial({
      color: 0xffffff,
      gradientMap: gradient,
      emissive: new THREE.Color(accent),
      emissiveIntensity: 0.25,
    })
    runeMaterials.push(petalMat)
    const petalGeo = new THREE.BoxGeometry(2.2, 0.4, 1.2)
    for (let i = 0; i < 10; i++) {
      const a = (i / 10) * Math.PI * 2
      const petal = new THREE.Mesh(petalGeo, petalMat)
      petal.position.set(Math.cos(a) * 4.2, 3.2, Math.sin(a) * 4.2)
      petal.rotation.y = -a
      group.add(petal)
    }
    // 尖顶
    const roof = new THREE.Mesh(new THREE.ConeGeometry(2.2, 3, 10), stone)
    roof.position.y = 4.6
    group.add(roof)
    this.scene.add(group)

    // 秘境花园：几棵低模树
    const leafMat = new THREE.MeshToonMaterial({ color: THEME_COLOR.verdant, gradientMap: gradient })
    this.treeMats.push(leafMat)
    const trunkMat = new THREE.MeshToonMaterial({ color: 0x8a5a3b, gradientMap: gradient })
    const trunkGeo = new THREE.CylinderGeometry(0.3, 0.4, 2, 6)
    const leafGeo = new THREE.ConeGeometry(1.4, 2.6, 7)
    const spots: [number, number][] = [[-18, -12], [-21, -9], [-16, -15], [-20, -15]]
    for (const [x, z] of spots) {
      const trunk = new THREE.Mesh(trunkGeo, trunkMat)
      trunk.position.set(x, 1, z)
      const leaf = new THREE.Mesh(leafGeo, leafMat)
      leaf.position.set(x, 3, z)
      leaf.castShadow = this.tier === 'high'
      this.scene.add(trunk, leaf)
    }

    // 委托板：两块立柱 + 板
    const wood = new THREE.MeshToonMaterial({ color: 0xa06a3c, gradientMap: gradient })
    const postGeo = new THREE.CylinderGeometry(0.18, 0.18, 2.2, 6)
    const p1 = new THREE.Mesh(postGeo, wood)
    p1.position.set(13.4, 1.1, 10)
    const p2 = new THREE.Mesh(postGeo, wood)
    p2.position.set(14.6, 1.1, 10)
    const board = new THREE.Mesh(new THREE.BoxGeometry(2, 1.3, 0.15), wood)
    board.position.set(14, 2.2, 10)
    this.scene.add(p1, p2, board)

    this.landmark = { group, runeMaterials }
  }

  private buildActors(gradient: THREE.DataTexture, tokens: { accent: number }): void {
    // 玩家：胶囊 + 头
    const bodyMat = new THREE.MeshToonMaterial({ color: 0x4f8fd0, gradientMap: gradient })
    const body = new THREE.Mesh(new THREE.CapsuleGeometry(0.5, 0.9, 4, 10), bodyMat)
    body.position.y = 1
    body.castShadow = true
    const head = new THREE.Mesh(new THREE.SphereGeometry(0.45, 12, 10), new THREE.MeshToonMaterial({ color: 0xffe0c2, gradientMap: gradient }))
    head.position.y = 2
    this.player.add(body, head)
    this.player.position.set(0, 0, 8)
    this.scene.add(this.player)

    // 跟随宠物（雏菊精灵：白球 + 花瓣）
    const petMat = new THREE.MeshToonMaterial({ color: 0xffffff, gradientMap: gradient })
    const petHead = new THREE.Mesh(new THREE.SphereGeometry(0.35, 10, 8), petMat)
    petHead.position.y = 0.8
    const petWing = new THREE.Mesh(new THREE.ConeGeometry(0.25, 0.6, 6), new THREE.MeshToonMaterial({ color: 0xf4c542, gradientMap: gradient }))
    petWing.position.y = 1.1
    this.pet.add(petHead, petWing)
    this.scene.add(this.pet)

    // 4 个编队精灵槽
    const elfColors = [0x9aa0b4, 0xf0c04a, 0x6aa0d8, 0xd88a9a]
    for (let i = 0; i < 4; i++) {
      const elf = new THREE.Group()
      const m = new THREE.MeshToonMaterial({
        color: elfColors[i],
        gradientMap: gradient,
        emissive: new THREE.Color(tokens.accent),
        emissiveIntensity: 0.1,
      })
      const body = new THREE.Mesh(new THREE.CapsuleGeometry(0.22, 0.4, 3, 8), m)
      body.position.y = 0.5
      elf.add(body)
      elf.visible = false
      this.elves.push(elf)
      this.scene.add(elf)
    }
  }

  // ── 外部 setter（Vue 层推入世界/引擎状态）─────────────────────────────

  setWorld(spec: WorldSpec): void {
    // 主题色驱动树叶与岛体（诚实：只换材质色，不造假事件）
    const c = THEME_COLOR[spec.theme] ?? THEME_COLOR.verdant
    for (const m of this.treeMats) m.color.setHex(c)
    this.scene.background = new THREE.Color(spec.weather === 'night' ? 0x1a2138 : 0xbfe3ff)
    if (spec.weather === 'fog') {
      this.scene.fog = new THREE.Fog(0xc9d6de, 20, 70)
    } else {
      this.scene.fog = null
    }
    this.setWeather(spec.weather)
  }

  setWeather(w: WeatherKind): void {
    if (w === 'rain' && !this.rain) {
      const n = this.tier === 'low' ? 80 : 220
      const pos = new Float32Array(n * 3)
      for (let i = 0; i < n; i++) {
        pos[i * 3] = (Math.random() - 0.5) * 60
        pos[i * 3 + 1] = Math.random() * 30
        pos[i * 3 + 2] = (Math.random() - 0.5) * 60
      }
      const g = new THREE.BufferGeometry()
      g.setAttribute('position', new THREE.BufferAttribute(pos, 3))
      this.rain = new THREE.Points(g, new THREE.PointsMaterial({ color: 0xaecbe8, size: 0.18 }))
      this.scene.add(this.rain)
    } else if (w !== 'rain' && this.rain) {
      this.scene.remove(this.rain)
      this.rain.geometry.dispose()
      this.rain = undefined
    }
  }

  setDayPhase(phase: 'day' | 'night'): void {
    const night = phase === 'night'
    this.sun.intensity = night ? 0.15 : 1.4
    this.sun.color.setHex(night ? 0x8899ff : 0xfff2d0)
    this.hemi.intensity = night ? 0.35 : 0.9
    if (!this.rain) this.scene.background = new THREE.Color(night ? 0x1a2138 : 0xbfe3ff)
  }

  setParty(count: number): void {
    this.elves.forEach((elf, i) => {
      elf.visible = i < count
    })
  }

  /**
   * 缓存命中率 → 萤火虫密度（蓝图 §2.1/§6.1：命中率 >80% 时萤火虫增多）。
   *
   * 只接受 0..100 的**真实**命中率；null（未连接/方法不存在）= 不生成，
   * 绝不用随机数假装「缓存很好」。低画质档封顶 90 只，守住桌面常驻预算。
   */
  setCacheHitRate(rate: number | null): void {
    const raw = rate == null || !Number.isFinite(rate) ? 0 : rate
    const bucket = raw >= 80 ? 300 : raw >= 50 ? 150 : raw >= 20 ? 60 : 0
    const count = this.tier === 'low' ? Math.min(bucket, 90) : bucket
    if (count === this.fireflyCount) return
    this.fireflyCount = count

    if (this.fireflies) {
      this.scene.remove(this.fireflies)
      this.fireflies.geometry.dispose()
      ;(this.fireflies.material as THREE.Material).dispose()
      this.fireflies = undefined
      this.fireflyBase = undefined
    }
    if (count <= 0) return

    const pos = new Float32Array(count * 3)
    for (let i = 0; i < count; i++) {
      const a = Math.random() * Math.PI * 2
      const r = 4 + Math.random() * 30
      pos[i * 3] = Math.cos(a) * r
      pos[i * 3 + 1] = 1 + Math.random() * 6
      pos[i * 3 + 2] = Math.sin(a) * r
    }
    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.BufferAttribute(pos.slice(), 3))
    this.fireflies = new THREE.Points(
      g,
      new THREE.PointsMaterial({
        color: 0xffe9a0,
        size: 0.22,
        transparent: true,
        opacity: 0.9,
        depthWrite: false,
      }),
    )
    this.fireflyBase = pos
    this.scene.add(this.fireflies)
  }

  /** 模式符文点灯：bloom/omni 时圣殿花瓣发光增强。 */
  setModeKind(kind: string | null): void {
    const lit = kind === 'bloom' || kind === 'omni-transcend'
    for (const m of this.landmark?.runeMaterials ?? []) {
      m.emissiveIntensity = lit ? 0.9 : 0.25
    }
  }

  // ── 输入 / 循环 ──────────────────────────────────────────────────────

  private onKeyDown = (e: KeyboardEvent): void => {
    const k = e.key.toLowerCase()
    if (['arrowup', 'arrowdown', 'arrowleft', 'arrowright', 'w', 'a', 's', 'd'].includes(k)) {
      this.keys.add(k)
    }
  }
  private onKeyUp = (e: KeyboardEvent): void => {
    this.keys.delete(e.key.toLowerCase())
  }

  private onResize = (): void => {
    const parent = this.renderer.domElement.parentElement
    if (!parent) return
    const w = parent.clientWidth || 800
    const h = parent.clientHeight || 500
    this.renderer.setSize(w, h, false)
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
  }

  private loop = (): void => {
    if (this.disposed) return
    this.raf = requestAnimationFrame(this.loop)
    const dt = Math.min(this.clock.getDelta(), 0.05)

    // 移动
    let dx = 0
    let dz = 0
    if (this.keys.has('w') || this.keys.has('arrowup')) dz -= 1
    if (this.keys.has('s') || this.keys.has('arrowdown')) dz += 1
    if (this.keys.has('a') || this.keys.has('arrowleft')) dx -= 1
    if (this.keys.has('d') || this.keys.has('arrowright')) dx += 1
    if (dx || dz) {
      const len = Math.hypot(dx, dz)
      const speed = this.reduceMotion ? 4 : 8
      this.player.position.x += (dx / len) * speed * dt
      this.player.position.z += (dz / len) * speed * dt
      // 约束在岛内
      const r = Math.hypot(this.player.position.x, this.player.position.z)
      if (r > 36) {
        this.player.position.x *= 36 / r
        this.player.position.z *= 36 / r
      }
      this.player.rotation.y = Math.atan2(dx, dz)
    }

    // 宠物跟随（缓动到玩家身后）
    const behind = new THREE.Vector3(this.player.position.x, 0, this.player.position.z + 1.6)
    this.pet.position.lerp(behind, 0.08)
    if (!this.reduceMotion) this.pet.position.y = Math.sin(this.clock.elapsedTime * 2) * 0.15

    // 编队精灵环绕跟随
    this.elves.forEach((elf, i) => {
      if (!elf.visible) return
      const a = this.clock.elapsedTime * 0.8 + (i * Math.PI * 2) / 4
      elf.position.set(
        this.player.position.x + Math.cos(a) * (1.4 + i * 0.3),
        0,
        this.player.position.z + Math.sin(a) * (1.4 + i * 0.3),
      )
    })

    // 雨下落
    if (this.rain) {
      const p = this.rain.geometry.getAttribute('position') as THREE.BufferAttribute
      for (let i = 0; i < p.count; i++) {
        let y = p.getY(i) - dt * 18
        if (y < 0) y = 28
        p.setY(i, y)
      }
      p.needsUpdate = true
    }

    // 萤火虫漂浮（缓存命中率越高，粒子越多；reduced-motion 时保持静止）
    if (this.fireflies && this.fireflyBase && !this.reduceMotion) {
      const p = this.fireflies.geometry.getAttribute('position') as THREE.BufferAttribute
      const t = this.clock.elapsedTime
      for (let i = 0; i < p.count; i++) {
        p.setY(i, this.fireflyBase[i * 3 + 1] + Math.sin(t * 0.8 + i * 0.7) * 0.35)
      }
      p.needsUpdate = true
    }

    // 相机跟随
    const targetCam = new THREE.Vector3(this.player.position.x, 8.5, this.player.position.z + 11)
    this.camera.position.lerp(targetCam, 0.06)
    this.camera.lookAt(this.player.position.x, 1, this.player.position.z)

    this.renderer.render(this.scene, this.camera)

    // HUD 采样
    this.frameCount++
    this.fpsTimer += dt
    if (this.fpsTimer >= 1) {
      this.hud.fps = Math.round(this.frameCount / this.fpsTimer)
      this.hud.drawCalls = this.renderer.info.render.calls
      this.onHud?.(this.hud)
      this.frameCount = 0
      this.fpsTimer = 0
    }
  }

  dispose(): void {
    this.disposed = true
    cancelAnimationFrame(this.raf)
    window.removeEventListener('keydown', this.onKeyDown)
    window.removeEventListener('keyup', this.onKeyUp)
    window.removeEventListener('resize', this.onResize)
    this.scene.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      if (mesh.geometry) mesh.geometry.dispose()
      const mat = mesh.material as THREE.Material | THREE.Material[] | undefined
      if (Array.isArray(mat)) mat.forEach((m) => m.dispose())
      else if (mat) mat.dispose()
    })
    this.renderer.dispose()
  }
}
