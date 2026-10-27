import * as THREE from 'three'
import {
  CLOUD_SEA_Y,
  HUT_HEIGHT,
  HUT_LENGTH,
  HUT_WIDTH,
  HUT_X,
  HUT_Z,
  dawnIntensity,
  hutBaseY,
  ridgeHeight,
  ropePathPoint,
  slopeUpness,
  sunDirection,
  sunElevation,
} from './ridgeMath'

/**
 * 玛格丽特 · 星夜 —— 三维山脊场景。
 *
 * 画面构成（全部程序化生成，不加载任何图片）：
 *   前景雪脊 + 结组而行的登山队 + 铜皮补给站 + 雏菊 + 云海 + 金字塔主峰 +
 *   **左半星空 / 右半晨曦** 的天光 + 缓缓跃出山脊的日出。
 *
 * 纪律：
 *   · 颜色只从主题令牌解析（不写死任何色值），换主题/换皮肤时场景自动跟着变；
 *   · 几何全部确定性生成（无随机数），同一台机器每次打开山形一致；
 *   · `prefers-reduced-motion` 时只渲染一帧；
 *   · 单场景面数控制在 ~5 万以内，纹理只用到 2 张小尺寸 Canvas 贴图。
 */

export interface RidgeSceneStats {
  running: boolean
  reducedMotion: boolean
  triangles: number
  dawn: number
  sunElevationDeg: number
}

const SKY_VERT = /* glsl */ `
  varying vec3 vDir;
  void main() {
    vDir = position;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
  }
`

const SKY_FRAG = /* glsl */ `
  varying vec3 vDir;
  uniform vec3 uZenithNight;
  uniform vec3 uZenithDawn;
  uniform vec3 uHorizonNight;
  uniform vec3 uHorizonDawn;
  uniform vec3 uSunColor;
  uniform vec3 uSunDir;
  uniform float uDawn;
  uniform float uTime;

  float hash13(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return fract((p.x + p.y) * p.z);
  }

  void main() {
    vec3 d = normalize(vDir);
    float h = clamp(d.y, 0.0, 1.0);
    float az = atan(d.x, -d.z);                 // -PI..PI，0 = 正前方
    // 左半是夜、右半是晨曦：用方位角做空间分割（不是按时间闪烁）
    float split = smoothstep(-0.75, 0.95, az);
    vec3 zenith = mix(uZenithNight, uZenithDawn, split * uDawn);
    vec3 horizon = mix(uHorizonNight, uHorizonDawn, split * uDawn);
    vec3 col = mix(horizon, zenith, pow(h, 0.62));

    // 银河：一条倾斜的淡带，只在夜侧可见
    vec3 bandAxis = normalize(vec3(0.34, 0.42, -0.84));
    float band = smoothstep(0.42, 0.0, abs(dot(d, bandAxis)));
    float bandNoise = hash13(floor(d * 90.0)) * 0.6 + 0.4;
    col += uZenithNight * 0.0 + vec3(0.32, 0.36, 0.48) * band * bandNoise * 0.16 * (1.0 - split * 0.9);

    // 星：哈希点阵，夜侧密集、晨曦侧几乎消失
    float starMask = (1.0 - split * 0.92) * smoothstep(0.02, 0.22, d.y);
    float n = hash13(floor(d * 260.0));
    float twinkle = 0.72 + 0.28 * sin(uTime * 1.6 + n * 51.0);
    float star = smoothstep(0.9972, 1.0, n) * twinkle;
    col += vec3(0.86, 0.91, 1.0) * star * starMask * 1.9;

    // 太阳光晕 + 地平线暖带
    float sunDot = max(dot(d, normalize(uSunDir)), 0.0);
    col += uSunColor * pow(sunDot, 42.0) * (0.55 + 0.85 * uDawn);
    col += uSunColor * pow(sunDot, 6.0) * 0.16 * uDawn;

    // 南侧（Po 平原）城市光穹：地平线上一条极淡的暖光（真实地理特征）
    float south = smoothstep(-0.02, -0.12, d.y) * smoothstep(-0.2, 1.2, az);
    col += uSunColor * south * 0.05;

    gl_FragColor = vec4(col, 1.0);
  }
`

function clamp01(v: number): number {
  return Math.min(1, Math.max(0, v))
}

export class RidgeScene {
  private readonly container: HTMLElement
  private readonly renderer: THREE.WebGLRenderer
  private readonly scene: THREE.Scene
  private readonly camera: THREE.PerspectiveCamera
  private readonly clock = new THREE.Clock()
  private readonly reducedMotion: boolean

  private skyMat!: THREE.ShaderMaterial
  private sunDisc!: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>
  private sunLight!: THREE.DirectionalLight
  private cloudLayers: Array<{ mesh: THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial>; speed: number }> = []
  private puffs: Array<{ sprite: THREE.Sprite; baseY: number }> = []
  private climbers: THREE.Group[] = []
  /** 前景雏菊的花冠：夜里收拢、日出张开（要逐帧改 scale）。 */
  private foregroundPetals: THREE.Object3D[] = []
  private rope!: THREE.Line<THREE.BufferGeometry, THREE.LineBasicMaterial>
  /** 从路径中段起步：t=0 时 5 个人全叠在左端、又偏出取景框，等于这一队不存在 */
  private teamPhase = 0.45
  /** 天光时钟：走真实时间（不受 dt 夹紧影响），240 s 一个日出轮回。 */
  private skyTime: number
  private pointer = new THREE.Vector2(0, 0)
  private raf = 0
  private disposed = false
  private resizeObserver: ResizeObserver | null = null
  private readonly onPointerMove = (e: PointerEvent): void => {
    const rect = this.container.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return
    this.pointer.set(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      ((e.clientY - rect.top) / rect.height) * 2 - 1,
    )
  }
  private palette: {
    night: THREE.Color
    nightTop: THREE.Color
    moon: THREE.Color
    aurora: THREE.Color
    warm: THREE.Color
    ember: THREE.Color
    snow: THREE.Color
    rock: THREE.Color
  }

  constructor(container: HTMLElement) {
    this.container = container
    this.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    this.palette = this.readPalette()
    // 调试/取证钩子：`?ridge-t=60` 直接把天光时钟拨到指定秒（0 夜 / 60 日出 / 120 高日）
    const debugT = Number(new URLSearchParams(window.location.search).get('ridge-t'))
    this.skyTime = Number.isFinite(debugT) && debugT > 0 ? Math.min(2_000_000, debugT) : 0

    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' })
    this.renderer.setPixelRatio(Math.min(1.5, window.devicePixelRatio || 1))
    this.renderer.setSize(container.clientWidth || 1, container.clientHeight || 1, false)
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping
    this.renderer.toneMappingExposure = 1.06
    this.renderer.domElement.className = 'marguerite-ridge-canvas'
    container.appendChild(this.renderer.domElement)

    this.scene = new THREE.Scene()
    this.scene.fog = new THREE.FogExp2(this.palette.night.clone().getHex(), 0.0032)

    this.camera = new THREE.PerspectiveCamera(40, this.aspect(), 0.5, 900)
    this.camera.position.set(0, 19, 56)
    this.camera.lookAt(0, 12, -34)

    this.buildSky()
    this.buildLights()
    this.buildRidge()
    this.buildCloudSea()
    this.buildHut()
    this.buildTeam()
    this.buildDaisies()

    // 背景层是 pointer-events:none 的，收不到自己的指针事件 → 监听 window 再按容器矩形换算
    window.addEventListener('pointermove', this.onPointerMove, { passive: true })
    // 告诉世界层：3D 山脊已接管背景（CSS 版的雪脊剪影要退场，避免两套轮廓打架）
    document.documentElement.dataset.ridge = '1'
    this.resizeObserver = new ResizeObserver(() => this.resize())
    this.resizeObserver.observe(container)
    this.update(this.skyTime, 0.016, true)
  }

  /** 主题令牌 → 场景色板。令牌用探针元素解析（支持 color-mix 等表达式）。 */
  private readPalette(): RidgeScene['palette'] {
    const probe = document.createElement('span')
    probe.style.display = 'none'
    document.body.appendChild(probe)
    const resolve = (name: string, fallback: string): THREE.Color => {
      probe.style.color = ''
      probe.style.color = `var(${name})`
      if (!probe.style.color) probe.style.color = `var(${fallback})`
      const rgb = getComputedStyle(probe).color
      probe.style.color = ''
      return new THREE.Color(rgb)
    }
    const night = resolve('--bg', '--bg-surface')
    const nightTop = resolve('--bg-surface', '--bg')
    const moon = resolve('--text', '--text-muted')
    const aurora = resolve('--accent-secondary', '--accent')
    const warm = resolve('--warn', '--accent')
    const ember = resolve('--danger', '--warn')
    const snow = resolve('--text', '--accent')
    const rock = resolve('--bg-surface-2', '--bg-surface')
    probe.remove()
    return { night, nightTop, moon, aurora, warm, ember, snow, rock }
  }

  private aspect(): number {
    const w = this.container.clientWidth || 1
    const h = this.container.clientHeight || 1
    return w / h
  }

  private buildSky(): void {
    const { night, nightTop, moon, warm } = this.palette
    const sun = sunDirection(0)
    this.skyMat = new THREE.ShaderMaterial({
      side: THREE.BackSide,
      depthWrite: false,
      vertexShader: SKY_VERT,
      fragmentShader: SKY_FRAG,
      uniforms: {
        uZenithNight: { value: night.clone().multiplyScalar(0.55) },
        uZenithDawn: { value: nightTop.clone().lerp(warm, 0.35) },
        uHorizonNight: { value: night.clone().lerp(moon, 0.08) },
        uHorizonDawn: { value: warm.clone().lerp(moon, 0.15) },
        uSunColor: { value: warm.clone() },
        uSunDir: { value: new THREE.Vector3(sun.x, sun.y, sun.z) },
        uDawn: { value: 0 },
        uTime: { value: 0 },
      },
    })
    const sky = new THREE.Mesh(new THREE.SphereGeometry(420, 40, 24), this.skyMat)
    sky.name = 'sky'
    sky.frustumCulled = false
    this.scene.add(sky)

    // 太阳本体：贴在山脊右侧的低空，随高度角缓慢升起
    // toneMapped=false（别被 ACES 压成灰饼）+ 柔边贴图（不要一枚硬边"纸圆片"）
    const disc = new THREE.Mesh(
      new THREE.PlaneGeometry(34, 34),
      new THREE.MeshBasicMaterial({
        map: this.makeGlowTexture(),
        color: warm.clone().lerp(moon, 0.3),
        transparent: true,
        opacity: 0.96,
        depthWrite: false,
        toneMapped: false,
        fog: false,
      }),
    )
    disc.name = 'sun-disc'
    disc.position.set(sun.x * 300, sun.y * 300, sun.z * 300)
    this.scene.add(disc)
    this.sunDisc = disc
  }

  private buildLights(): void {
    const { night, moon, warm, aurora } = this.palette
    const hemi = new THREE.HemisphereLight(moon.clone().lerp(aurora, 0.35), night, 0.62)
    this.scene.add(hemi)
    const dir = new THREE.DirectionalLight(warm.clone(), 1.15)
    dir.position.set(40, 20, 10)
    this.scene.add(dir)
    this.sunLight = dir
    const fill = new THREE.DirectionalLight(moon.clone(), 0.42)
    // 月光从山后偏左打过来：雪脊迎光的一侧才有一条亮边，不至于整面刷成灰板
    fill.position.set(-70, 34, -80)
    this.scene.add(fill)
  }

  private buildRidge(): void {
    const width = 300
    const depth = 220
    const segX = 180
    const segZ = 132
    const geo = new THREE.PlaneGeometry(width, depth, segX, segZ)
    geo.rotateX(-Math.PI / 2)
    const pos = geo.attributes.position as THREE.BufferAttribute
    const colors = new Float32Array(pos.count * 3)
    const snowColor = this.palette.snow
    const rockColor = this.palette.rock
    const iceColor = this.palette.aurora
    const shadowColor = this.palette.night

    for (let i = 0; i < pos.count; i += 1) {
      const x = pos.getX(i)
      // 注意：这里不能再对 z 做整体偏移。地形一旦平移，而补给站/登山队/雏菊仍按
      // `ridgeHeight(x, z)` 落位，就会差出十几米 —— 实测现象是登山队整队"埋进雪里"。
      const z = pos.getZ(i)
      const y = ridgeHeight(x, z)
      pos.setY(i, y)
      const up = slopeUpness(x, z)
      // 缓坡积雪、陡壁裸岩；越靠近云海越冷（冰川蓝），越靠近日出侧越暖
      // 加一层确定性细噪声：远看是"雪的斑驳"，不是一块塑料布
      const grain = Math.sin(x * 0.63 + z * 0.41) * 0.5 + Math.sin(x * 1.27 - z * 0.88) * 0.5
      const snowMix = clamp01((up - 0.4) / 0.4 + grain * 0.1) * 0.88
      const altMix = clamp01((y - CLOUD_SEA_Y) / 18)
      const c = rockColor.clone().lerp(snowColor, snowMix * (0.42 + 0.58 * altMix))
      const iceMix = clamp01((0.62 - up) / 0.5) * clamp01((4 - y) / 8)
      c.lerp(iceColor, iceMix * 0.35)
      c.lerp(shadowColor, clamp01((y - 2) / 26) * 0.18)
      // 迎日出的一侧（+x）微微回暖，让"一半夜一半晨"落到山体上
      const warmSide = clamp01((x - 10) / 70) * clamp01(y / 16)
      c.lerp(this.palette.warm, warmSide * 0.07)
      colors[i * 3] = c.r
      colors[i * 3 + 1] = c.g
      colors[i * 3 + 2] = c.b
    }
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))
    geo.computeVertexNormals()
    const mat = new THREE.MeshStandardMaterial({
      vertexColors: true,
      roughness: 0.86,
      metalness: 0.02,
      flatShading: false,
    })
    const mesh = new THREE.Mesh(geo, mat)
    mesh.name = 'ridge'
    this.scene.add(mesh)
  }

  /** 云海贴图：一张 256² 的柔斑 Canvas，两层反向漂移，再叠 10 个近景云团。 */
  private makeCloudTexture(seed: number): THREE.CanvasTexture {
    const size = 256
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    if (!ctx) return new THREE.CanvasTexture(canvas)
    ctx.clearRect(0, 0, size, size)
    let s = seed
    const rand = (): number => {
      s = (s * 1664525 + 1013904223) % 4294967296
      return s / 4294967296
    }
    for (let i = 0; i < 90; i += 1) {
      const cx = rand() * size
      const cy = rand() * size
      const r = 18 + rand() * 58
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r)
      const a = 0.06 + rand() * 0.16
      g.addColorStop(0, `rgba(255,255,255,${a.toFixed(3)})`)
      g.addColorStop(1, 'rgba(255,255,255,0)')
      ctx.fillStyle = g
      ctx.beginPath()
      ctx.arc(cx, cy, r, 0, Math.PI * 2)
      ctx.fill()
    }
    const tex = new THREE.CanvasTexture(canvas)
    tex.wrapS = THREE.RepeatWrapping
    tex.wrapT = THREE.RepeatWrapping
    tex.repeat.set(3, 2)
    return tex
  }

  private buildCloudSea(): void {
    const tint = this.palette.moon.clone().lerp(this.palette.aurora, 0.25)
    const layers: Array<{ y: number; opacity: number; scale: number; speed: number; seed: number }> = [
      { y: CLOUD_SEA_Y + 0.2, opacity: 0.85, scale: 1, speed: 0.0045, seed: 7 },
      { y: CLOUD_SEA_Y + 2.2, opacity: 0.38, scale: 1.35, speed: -0.0072, seed: 31 },
    ]
    for (const layer of layers) {
      const geo = new THREE.PlaneGeometry(520 * layer.scale, 340 * layer.scale)
      geo.rotateX(-Math.PI / 2)
      const mat = new THREE.MeshBasicMaterial({
        map: this.makeCloudTexture(layer.seed),
        color: tint.clone(),
        transparent: true,
        opacity: layer.opacity,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      })
      const mesh = new THREE.Mesh(geo, mat)
      mesh.position.set(0, layer.y, -60)
      mesh.name = 'cloud-sea'
      this.scene.add(mesh)
      this.cloudLayers.push({ mesh, speed: layer.speed })
    }

    // 远景云团：只放在主峰两侧的云海上（放进前景会糊成一块灰板，反而看不出"海"）
    const puffTex = this.makeCloudTexture(101)
    for (let i = 0; i < 8; i += 1) {
      const mat = new THREE.SpriteMaterial({
        map: puffTex,
        color: tint.clone(),
        transparent: true,
        opacity: 0.14 + (i % 3) * 0.05,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      })
      const sprite = new THREE.Sprite(mat)
      const baseY = CLOUD_SEA_Y + 0.4 + (i % 3) * 1.1
      sprite.position.set(-126 + i * 36, baseY, -72 - ((i * 13) % 4) * 16)
      sprite.scale.set(24 + (i % 3) * 10, 8 + (i % 3) * 4, 1)
      sprite.name = 'cloud-puff'
      this.scene.add(sprite)
      this.puffs.push({ sprite, baseY })
    }
  }

  /**
   * 补给站：20.5×8×7 m 的铜皮长条 + 浅坡雪顶 + 木露台 + 珍珠灯串 + 气象杆。
   * 参照 Capanna Regina Margherita（1980 年重建的现屋）：整体铜皮包覆、长边木露台、
   * 屋顶常年积雪、2000 年起是欧洲最高的气象站。
   */
  private buildHut(): void {
    const group = new THREE.Group()
    group.name = 'hut'
    const baseY = hutBaseY()
    group.position.set(HUT_X, baseY, HUT_Z)

    // 铜皮外墙：夜里是接近黑的暖棕，只有窗光一带被提亮（不要做成"木屋"）
    const copper = this.palette.warm.clone().lerp(this.palette.rock, 0.72)
    const bodyMat = new THREE.MeshStandardMaterial({ color: copper, roughness: 0.58, metalness: 0.42 })
    const body = new THREE.Mesh(new THREE.BoxGeometry(HUT_LENGTH, HUT_HEIGHT, HUT_WIDTH), bodyMat)
    body.position.y = HUT_HEIGHT / 2
    group.add(body)

    const metalMat = new THREE.MeshStandardMaterial({ color: this.palette.rock.clone().lerp(this.palette.moon, 0.2), roughness: 0.7, metalness: 0.3 })
    const roofMat = new THREE.MeshStandardMaterial({ color: this.palette.snow.clone(), roughness: 0.94 })

    // 双坡雪顶：两块微倾的板在屋脊相接（浅坡才挂得住雪，陡坡反而存不住）
    const halfSpan = HUT_WIDTH / 2 + 0.9
    const rise = 1.7
    const pitch = Math.atan2(rise, halfSpan)
    for (const side of [1, -1]) {
      const panel = new THREE.Mesh(
        new THREE.BoxGeometry(HUT_LENGTH + 1.6, 0.42, Math.hypot(halfSpan, rise)),
        roofMat,
      )
      panel.position.set(0, HUT_HEIGHT + rise / 2, (side * halfSpan) / 2)
      panel.rotation.x = -side * pitch
      group.add(panel)
    }
    const ridgeCap = new THREE.Mesh(new THREE.BoxGeometry(HUT_LENGTH + 2, 0.3, 0.9), roofMat)
    ridgeCap.position.set(0, HUT_HEIGHT + rise + 0.06, 0)
    group.add(ridgeCap)

    // 烟囱：最冷的夜里炉子一直烧着，是"有人在"的信号
    const stack = new THREE.Mesh(new THREE.CylinderGeometry(0.26, 0.28, 2.6, 10), metalMat)
    stack.position.set(-HUT_LENGTH / 2 + 4.2, HUT_HEIGHT + rise + 1.1, 1.6)
    group.add(stack)

    // 长边木露台
    const deckMat = new THREE.MeshStandardMaterial({
      color: this.palette.rock.clone().lerp(this.palette.warm, 0.16),
      roughness: 0.92,
    })
    const deck = new THREE.Mesh(new THREE.BoxGeometry(HUT_LENGTH + 0.6, 0.26, 2.6), deckMat)
    deck.position.set(0, 2.3, HUT_WIDTH / 2 + 1.2)
    group.add(deck)

    // 露台栏杆 + 珍珠灯串（向"珍珠女王"致意：一串暖白光点）
    const rail = new THREE.Mesh(new THREE.BoxGeometry(HUT_LENGTH + 0.6, 0.12, 0.12), metalMat)
    rail.position.set(0, 3.2, HUT_WIDTH / 2 + 2.3)
    group.add(rail)
    const pearlMat = new THREE.MeshStandardMaterial({
      color: this.palette.moon.clone(),
      emissive: this.palette.warm.clone().multiplyScalar(0.7),
      emissiveIntensity: 1.4,
      roughness: 0.3,
    })
    for (let i = 0; i < 9; i += 1) {
      const pearl = new THREE.Mesh(new THREE.SphereGeometry(0.22, 10, 8), pearlMat)
      pearl.position.set(-HUT_LENGTH / 2 + (i * HUT_LENGTH) / 8, 3.05, HUT_WIDTH / 2 + 2.3)
      group.add(pearl)
    }

    // 窗：暖光片（夜里最抓眼的"人味"）
    const winMat = new THREE.MeshBasicMaterial({ color: this.palette.warm.clone().lerp(this.palette.moon, 0.22) })
    for (let i = 0; i < 4; i += 1) {
      const win = new THREE.Mesh(new THREE.PlaneGeometry(1.6, 1.1), winMat)
      win.position.set(-6.6 + i * 4.1, 4.2, HUT_WIDTH / 2 + 0.03)
      group.add(win)
    }
    const windowLight = new THREE.PointLight(this.palette.warm.clone(), 7, 22, 2)
    windowLight.position.set(0, 4.4, HUT_WIDTH / 2 + 3)
    group.add(windowLight)

    // 气象杆 + 风杯（2000 年起这里是欧洲最高气象站；不是老屋那座 1899 观测塔）
    const mast = new THREE.Mesh(new THREE.CylinderGeometry(0.07, 0.07, 3.8, 8), metalMat)
    mast.position.set(HUT_LENGTH / 2 - 1.6, HUT_HEIGHT + rise + 1.9, 0)
    group.add(mast)
    const anemometer = new THREE.Mesh(new THREE.TorusGeometry(0.4, 0.035, 6, 16), metalMat)
    anemometer.position.set(HUT_LENGTH / 2 - 1.6, HUT_HEIGHT + rise + 3.8, 0)
    anemometer.rotation.x = Math.PI / 2
    group.add(anemometer)

    this.scene.add(group)
  }

  /** 登山队：5 人结组（腿/躯干/手臂/头/头盔/背包/冰镐/头灯）。
   *  配色不是纯剪影：他们站在背光的雪坡上，纯黑会和山体糊成一片，所以外套取主色
   *  的低饱和钢青、背包取雏菊金，头灯是唯一的高光点。 */
  private buildTeam(): void {
    const shell = this.palette.aurora.clone().lerp(this.palette.night, 0.55)
    const bodyMat = new THREE.MeshStandardMaterial({ color: shell, roughness: 0.85, emissive: shell.clone().multiplyScalar(0.08) })
    const legMat = new THREE.MeshStandardMaterial({ color: this.palette.rock.clone().lerp(this.palette.night, 0.45), roughness: 0.9 })
    const packMat = new THREE.MeshStandardMaterial({
      color: this.palette.warm.clone().lerp(this.palette.night, 0.62),
      roughness: 0.88,
    })
    const lampMat = new THREE.MeshBasicMaterial({ color: this.palette.moon.clone() })
    const skinMat = new THREE.MeshStandardMaterial({
      color: this.palette.moon.clone().lerp(this.palette.rock, 0.58),
      roughness: 0.92,
    })
    const torso = new THREE.CapsuleGeometry(0.3, 0.72, 4, 10)
    const limb = new THREE.CapsuleGeometry(0.12, 0.52, 4, 8)
    const headGeo = new THREE.SphereGeometry(0.25, 12, 10)
    const helmetGeo = new THREE.SphereGeometry(0.28, 12, 8, 0, Math.PI * 2, 0, Math.PI / 1.7)
    for (let i = 0; i < 5; i += 1) {
      const g = new THREE.Group()
      g.name = `climber-${i}`
      const body = new THREE.Mesh(torso, bodyMat)
      body.position.y = 1.5
      g.add(body)
      for (const side of [-1, 1]) {
        const leg = new THREE.Mesh(limb, legMat)
        leg.position.set(side * 0.17, 0.62, 0)
        leg.rotation.z = side * 0.06
        g.add(leg)
        const arm = new THREE.Mesh(limb, bodyMat)
        arm.position.set(side * 0.42, 1.62, 0.04)
        arm.rotation.z = -side * 0.18
        g.add(arm)
      }
      const head = new THREE.Mesh(headGeo, skinMat)
      head.position.y = 2.28
      g.add(head)
      const helmet = new THREE.Mesh(helmetGeo, bodyMat)
      helmet.position.y = 2.32
      g.add(helmet)
      const pack = new THREE.Mesh(new THREE.BoxGeometry(0.52, 0.8, 0.36), packMat)
      pack.position.set(0, 1.62, -0.34)
      g.add(pack)
      const axe = new THREE.Mesh(new THREE.CylinderGeometry(0.03, 0.03, 1.3, 5), legMat)
      axe.position.set(0.44, 1.25, 0.14)
      axe.rotation.z = 0.22
      g.add(axe)
      // 头灯：凌晨结组攀登的真实特征，也是画面里唯一的高光点
      const lamp = new THREE.Mesh(new THREE.SphereGeometry(0.07, 8, 6), lampMat)
      lamp.position.set(0, 2.36, 0.24)
      g.add(lamp)
      g.scale.setScalar(1.15) // 模型本体约 2.55 m：略放大，面板压暗后仍能读出"结组"
      this.scene.add(g)
      this.climbers.push(g)
    }

    const ropeGeo = new THREE.BufferGeometry()
    ropeGeo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(5 * 3), 3))
    const rope = new THREE.Line(
      ropeGeo,
      new THREE.LineBasicMaterial({ color: this.palette.moon.clone().lerp(this.palette.ember, 0.5), transparent: true, opacity: 0.55 }),
    )
    rope.name = 'rope'
    this.scene.add(rope)
    this.rope = rope
  }

  /** 雏菊：真实的 Bellis perennis 夜里会闭合，所以远处是闭合花苞，只有窗光附近开几朵。 */
  private buildDaisies(): void {
    const budGeo = new THREE.SphereGeometry(0.1, 8, 6)
    const budMat = new THREE.MeshStandardMaterial({ color: this.palette.moon.clone().lerp(this.palette.snow, 0.5), roughness: 0.85 })
    const buds = new THREE.InstancedMesh(budGeo, budMat, 26)
    const m = new THREE.Matrix4()
    let s = 12345
    const rand = (): number => {
      s = (s * 1103515245 + 12345) % 2147483648
      return s / 2147483648
    }
    for (let i = 0; i < 26; i += 1) {
      const x = HUT_X + (rand() - 0.5) * 26
      const z = HUT_Z + 5 + rand() * 7
      const y = ridgeHeight(x, z) + 0.12
      m.makeTranslation(x, y, z)
      buds.setMatrixAt(i, m)
    }
    buds.name = 'daisies-closed'
    this.scene.add(buds)

    // 窗光下的几朵是开着的：白色舌状花瓣 + 金黄花心
    const petal = new THREE.Mesh(
      new THREE.CircleGeometry(0.16, 10),
      new THREE.MeshBasicMaterial({ color: this.palette.snow.clone(), side: THREE.DoubleSide }),
    )
    const heart = new THREE.Mesh(
      new THREE.CircleGeometry(0.05, 8),
      new THREE.MeshBasicMaterial({ color: this.palette.warm.clone() }),
    )
    for (let i = 0; i < 6; i += 1) {
      const x = HUT_X + 1.5 + i * 0.55
      const z = HUT_Z + HUT_WIDTH / 2 + 2.6
      const y = ridgeHeight(x, z) + 0.16
      const flower = petal.clone()
      flower.position.set(x, y, z)
      flower.rotation.x = -Math.PI / 2
      this.scene.add(flower)
      const core = heart.clone()
      core.position.set(x, y + 0.01, z)
      core.rotation.x = -Math.PI / 2
      this.scene.add(core)
    }

    // 前景雏菊：贴着镜头侧的雪坡（画面下缘能看清的那一排）。
    // 真实的 Bellis perennis 日出张开、日落后收拢 —— 花瓣组随 dawn 放大缩小。
    const daisyTex = this.makeDaisyTexture()
    const headGeo = new THREE.PlaneGeometry(1.05, 1.05)
    const headMat = new THREE.MeshBasicMaterial({
      map: daisyTex,
      transparent: true,
      alphaTest: 0.35,
      side: THREE.DoubleSide,
    })
    const stemMat = new THREE.MeshStandardMaterial({
      color: this.palette.night.clone().lerp(this.palette.rock, 0.6),
      roughness: 0.95,
    })
    const stemGeo = new THREE.CylinderGeometry(0.018, 0.026, 0.52, 5)
    for (let i = 0; i < 7; i += 1) {
      const x = -32 + i * 3.4
      const z = 15 + ((i * 5) % 3) * 1.2
      const ground = ridgeHeight(x, z)
      const stem = new THREE.Mesh(stemGeo, stemMat)
      stem.position.set(x, ground + 0.26, z)
      stem.rotation.z = (i % 3 - 1) * 0.14
      this.scene.add(stem)
      const head = new THREE.Mesh(headGeo, headMat)
      head.position.set(x, ground + 0.54, z)
      head.rotation.set(-Math.PI / 2 + 0.5, 0, i * 0.8)
      this.scene.add(head)
      this.foregroundPetals.push(head)
    }
  }

  /** 日轮贴图：中心实、边缘化开（真实太阳隔着厚大气层就是一团带晕的光）。 */
  private makeGlowTexture(): THREE.CanvasTexture {
    const size = 64
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    const tex = new THREE.CanvasTexture(canvas)
    if (!ctx) return tex
    const c = size / 2
    const g = ctx.createRadialGradient(c, c, 0, c, c, c)
    g.addColorStop(0, 'rgba(255,255,255,1)')
    g.addColorStop(0.36, 'rgba(255,255,255,0.98)')
    g.addColorStop(0.52, 'rgba(255,255,255,0.42)')
    g.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = g
    ctx.fillRect(0, 0, size, size)
    tex.colorSpace = THREE.SRGBColorSpace
    return tex
  }

  /** 雏菊贴图：15 枚白色舌状花 + 金黄管状花心（尺寸小，64² 足够）。 */
  private makeDaisyTexture(): THREE.CanvasTexture {
    const size = 64
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    const tex = new THREE.CanvasTexture(canvas)
    if (!ctx) return tex
    const c = size / 2
    for (let i = 0; i < 15; i += 1) {
      ctx.save()
      ctx.translate(c, c)
      ctx.rotate((i / 15) * Math.PI * 2)
      ctx.beginPath()
      ctx.ellipse(0, -c * 0.6, c * 0.15, c * 0.36, 0, 0, Math.PI * 2)
      ctx.fillStyle = '#eef2fb'
      ctx.fill()
      ctx.restore()
    }
    ctx.beginPath()
    ctx.arc(c, c, c * 0.3, 0, Math.PI * 2)
    ctx.fillStyle = '#f2c14a'
    ctx.fill()
    tex.colorSpace = THREE.SRGBColorSpace
    return tex
  }

  private update(t: number, dt: number, force = false): void {
    if (this.disposed) return
    const dawn = dawnIntensity(t)
    const sun = sunDirection(t)
    const sunPos = new THREE.Vector3(sun.x, sun.y, sun.z)

    this.skyMat.uniforms.uDawn.value = dawn
    this.skyMat.uniforms.uTime.value = t
    ;(this.skyMat.uniforms.uSunDir.value as THREE.Vector3).copy(sunPos)

    this.sunDisc.position.copy(sunPos).multiplyScalar(300)
    ;(this.sunDisc.material as THREE.MeshBasicMaterial).color.copy(
      // 日轮本体要压过周围那圈光晕，否则中心会比晕更暗，看着像个"孔"
      this.palette.moon.clone().lerp(this.palette.warm, 0.55 + dawn * 0.35),
    )

    this.sunLight.position.copy(sunPos).multiplyScalar(80)
    // 白天是日光的暖白；太阳还在地平线下时这盏灯其实是月光（冷），
    // 否则夜里的迎光面会被染成一块红板
    this.sunLight.color.copy(this.palette.moon.clone().lerp(this.palette.warm, dawn))
    this.sunLight.intensity = 0.18 + dawn * 1.7

    for (const layer of this.cloudLayers) {
      const map = layer.mesh.material.map
      if (map) {
        // 用 dt 推进而不是"每帧固定步长"：不同刷新率/掉帧时云速一致
        map.offset.x = (map.offset.x + layer.speed * dt) % 1
        map.offset.y = (map.offset.y + layer.speed * 0.4 * dt) % 1
      }
      layer.mesh.material.color.copy(
        this.palette.moon.clone().lerp(this.palette.warm, dawn * 0.55).lerp(this.palette.aurora, 0.12),
      )
    }

    // 云团缓慢起伏（"云海翻涌"）
    for (let i = 0; i < this.puffs.length; i += 1) {
      const puff = this.puffs[i]
      puff.sprite.position.y = puff.baseY + Math.sin(t * 0.22 + i) * 0.35
    }

    // 雏菊：日出张开（夜间是合拢的花苞）
    const open = 0.32 + 0.68 * dawn
    for (const head of this.foregroundPetals) head.scale.setScalar(open)

    // 登山队：沿肩脊行进 + 轻微上下起伏；绳索逐帧重算（5 个点，成本可忽略）
    this.teamPhase += dt * 0.035
    if (this.teamPhase > 1.35) this.teamPhase = -0.35
    const ropePos = this.rope.geometry.attributes.position as THREE.BufferAttribute
    for (let i = 0; i < this.climbers.length; i += 1) {
      const p = ropePathPoint(this.teamPhase - i * 0.08)
      const bob = Math.sin(t * 2.4 + i * 0.9) * 0.06
      this.climbers[i].position.set(p.x, p.y + 0.05 + bob, p.z)
      this.climbers[i].rotation.y = -Math.PI / 2 + Math.sin(t * 0.8 + i) * 0.04
      ropePos.setXYZ(i, p.x, p.y + 1.45 + bob, p.z)
    }
    ropePos.needsUpdate = true

    // 相机：缓慢漂移 + 指针视差（克制：总位移不超过 1.5 m）
    const px = this.pointer.x * 1.4
    const py = this.pointer.y * 0.7
    this.camera.position.x = px + Math.sin(t * 0.05) * 0.9
    this.camera.position.y = 19 - py + Math.sin(t * 0.037) * 0.25
    this.camera.lookAt(0, 12, -34)

    if (force || !this.reducedMotion) this.renderer.render(this.scene, this.camera)

    this.container.dataset.margueriteRidge = this.reducedMotion ? 'static' : 'running'
    this.container.dataset.dawn = dawn.toFixed(3)
    this.container.dataset.sunElevation = ((sunElevation(t) * 180) / Math.PI).toFixed(2)
  }

  start(): void {
    if (this.raf || this.disposed) return
    if (this.reducedMotion) {
      this.update(this.skyTime + 6, 0.016, true)
      return
    }
    const loop = (): void => {
      if (this.disposed) return
      const raw = this.clock.getDelta()
      const dt = Math.min(0.05, raw)
      // 天光走真实秒（低帧率机器上太阳不会"慢放"），位移类动效走夹紧后的 dt（掉帧不跳变）
      this.skyTime += Math.min(0.5, raw)
      this.update(this.skyTime, dt)
      this.raf = requestAnimationFrame(loop)
    }
    this.raf = requestAnimationFrame(loop)
  }

  stop(): void {
    if (this.raf) cancelAnimationFrame(this.raf)
    this.raf = 0
  }

  stats(): RidgeSceneStats {
    let triangles = 0
    this.scene.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      const geo = mesh.geometry as THREE.BufferGeometry | undefined
      if (geo && typeof geo.index?.count === 'number') triangles += geo.index.count / 3
      else if (geo && typeof geo.attributes?.position?.count === 'number') {
        triangles += geo.attributes.position.count / 3
      }
    })
    return {
      running: this.raf !== 0,
      reducedMotion: this.reducedMotion,
      triangles: Math.round(triangles),
      dawn: Number(this.container.dataset.dawn ?? 0),
      sunElevationDeg: Number(this.container.dataset.sunElevation ?? 0),
    }
  }

  private resize(): void {
    if (this.disposed) return
    const w = this.container.clientWidth || 1
    const h = this.container.clientHeight || 1
    this.renderer.setSize(w, h, false)
    this.camera.aspect = w / h
    this.camera.updateProjectionMatrix()
    this.renderer.render(this.scene, this.camera)
  }

  dispose(): void {
    this.disposed = true
    this.stop()
    delete document.documentElement.dataset.ridge
    this.resizeObserver?.disconnect()
    this.resizeObserver = null
    this.container.removeEventListener('pointermove', this.onPointerMove)
    window.removeEventListener('pointermove', this.onPointerMove)
    this.scene.traverse((obj) => {
      const mesh = obj as THREE.Mesh
      const geo = mesh.geometry as THREE.BufferGeometry | undefined
      if (geo && !geo.userData.shared) geo.dispose()
      const mat = (mesh as unknown as { material?: THREE.Material | THREE.Material[] }).material
      if (Array.isArray(mat)) mat.forEach((m) => m.dispose())
      else if (mat) mat.dispose()
    })
    this.renderer.dispose()
    if (this.renderer.domElement.parentElement === this.container) {
      this.container.removeChild(this.renderer.domElement)
    }
  }
}
