/**
 * 岛屿模型规格（数据驱动建模）—— 「形状即数据」在 3D 工作空间上的延续。
 *
 * - 全部几何尺寸、配色、光照曲线、性能预算都收敛在 `IslandModelSpec` 里；
 *   渲染器（Island3D）与几何构建器（islandBuilders）不得写死数值。
 * - 新增皮肤 = 追加一份 Spec（`ISLAND_SPECS`），热插拔无需改渲染代码。
 * - `validateIslandSpec` 是纯校验：坏 Spec 在构造渲染器前就被拒绝（诚实渲染，不带病启动）。
 * - 颜色是 Three.js 数据层常量（`0xRRGGBB` 整数），不进入 CSS 着色管道；
 *   UI 侧的主题色仍走 CSS 语义 token（由 readIslandTheme 从计算样式读取）。
 */

import { ISLAND_KINDS, type IslandEntityKind } from './islandContract'

/**
 * 渲染质量档（由 detectIslandRuntime 判定）。
 *
 * 三档而不是两档：岛屿此前只有「全开 / 全关」，中端机（WebGL2 可用但核心数/内存偏低）
 * 与「只有 WebGL1 的老机」被判成同一档 —— 只能拿全部阴影+抗锯齿+2× 采样，或全部关掉。
 * `medium` 保留投影层次（岛屿的「温润感」一半来自阴影与灯火），只降抗锯齿/采样率/
 * 装饰密度/灯火增益，与桌宠 `PET_QUALITY.medium` 同一口径。
 */
export type IslandQualityTier = 'high' | 'medium' | 'low'

/** 档位次序（单一事实源：校验与档位推导都读它，避免各处各写一份列表） */
export const ISLAND_QUALITY_TIERS: readonly IslandQualityTier[] = ['high', 'medium', 'low']

/** 屋顶形态（决定建筑轮廓，2D 地图与 3D 共用同一枚举） */
export type IslandRoofStyle = 'gable' | 'dome' | 'cone' | 'flat' | 'beacon'

export interface IslandBuildingTemplate {
  /** 地基尺寸（米） */
  width: number
  depth: number
  height: number
  floors: number
  roof: IslandRoofStyle
  roofHeight: number
  wallColor: number
  roofColor: number
  trimColor: number
  /** 灯笼/窗口发光色（活动时点亮） */
  glowColor: number
  /** 窗户层数（0 = 无窗，如仓库/哨所） */
  windowRows: number
  /** 该建筑允许的点光源上限（再受 quality.lights 全局约束） */
  maxLights: number
}

export interface IslandQualityBudget {
  shadows: boolean
  antialias: boolean
  maxPixelRatio: number
  /** 装饰物数量系数 0..1（树/石/芦苇按此裁剪） */
  props: number
  /** 同屏粒子（行进的货箱/浪花）总量 */
  particles: number
  /**
   * 建筑装饰细节实例预算（记忆塔光环光点/书页鳍片等）。
   * 渲染层不得写死：按此字段经 athenaeumDetail 推导，低档自动减细节。
   */
  ornaments: number
  /** 点光源全局上限 */
  lights: number
  /**
   * 灯火点光增益（候选强度 × 此系数）。此前是渲染层的 `quality === 'high' ? 10 : 6`
   * 二元分档 —— 三档化后那种写法会把 medium 静默压成 low，故收敛到预算表。
   */
  lightGain: number
  /** 星空星点数量与点尺寸（同上：此前后者写死为 `high ? 2.6 : 2`） */
  stars: number
  starSize: number
  /**
   * WebGL 上下文功耗偏好。中端机取 `low-power`：为有限的画质收益唤醒独显不划算，
   * 也与「低档省电」同一取向（只有 high 明确要独显）。
   */
  powerPreference: WebGLPowerPreference
  /** 水面径向分段 */
  waterSegments: number
  /** 帧率上限（低档省电） */
  targetFps: number
  /** 地形径向分段 */
  terrainDetail: number
}

export interface IslandModelSpec {
  id: string
  label: string
  terrain: {
    /** 岛屿草地面半径 */
    radius: number
    /** 台地厚度（草顶 y=0，向下延伸） */
    height: number
    /** 岸线抖动强度 0..1（确定性哈希扰动，非随机数） */
    jitter: number
    /** 沙滩环宽度 */
    beachWidth: number
    /** 岛底岩体深度 */
    rockBase: number
  }
  water: {
    /** 水面 y 高度（低于草顶、高于岩底） */
    level: number
    radius: number
    waveHeight: number
    waveLength: number
    waveSpeed: number
    opacity: number
  }
  layout: {
    /** 工坊环半径 */
    coreRadius: number
    /** 哨所环半径增量 */
    ringGap: number
    /** 间距系数（越大越疏） */
    plotGap: number
    /** 栈桥长度 */
    pierLength: number
    /** 灯塔离轴距离 */
    beaconRadius: number
    /** 记忆塔离轴距离 */
    athenaeumRadius: number
    /** 码头方位角（弧度） */
    quayAngle: number
  }
  buildings: Record<IslandEntityKind, IslandBuildingTemplate>
  props: { trees: number; rocks: number; reeds: number; clouds: number }
  lighting: {
    /** 一整天（含夜）的秒数 */
    dayLength: number
    sunRadius: number
    sunHeight: number
    dayColor: number
    duskColor: number
    nightColor: number
    moonColor: number
    ambientDay: number
    ambientNight: number
    /**
     * 逆光补光强度：从太阳**对侧**斜打，专门照亮背离主光的侧壁/悬崖面。
     * 岛缘侧壁的法线朝外朝下，N·L ≈ 0，只有半球光的暗侧 → 阴影侧会压得偏重。
     * 补光不投影（避免与主光方向矛盾的第二重影子）。
     *
     * 实测（关掉自动昼夜固定日照后 A/B，同一画布区域）：
     *   0     → median 150.5  mean 123.2  暗部均值 87.7
     *   0.62  → median 165.4  mean 137.5  暗部均值 99.8   （暗部 +13.7%）
     * 即 0.62 确实柔化了阴影侧，而白天受光面本已接近饱和、几乎不受影响（主光 2.32 : 补光 0.62 ≈ 3.7:1）。
     */
    fillIntensity: number
    lanternColor: number
    lanternIntensity: number
    fogNear: number
    fogFar: number
    beamColor: number
  }
  palette: {
    grass: number
    grassDark: number
    sand: number
    rock: number
    water: number
    /**
     * 近岸浅滩色（青绿）：与 `water`/`waterDeep` 一起构成**水体径向色阶**。
     * 水面占画面约 85%，这条「浅滩青绿 → 主体海色 → 远海靛蓝」的层次
     * 是《我们的岛》唯美海面的核心；缺了它整片水只剩单色 + 雾。
     */
    waterShallow: number
    waterDeep: number
    wood: number
    woodDark: number
    stone: number
    cloth: number
    cloud: number
    /** 花树冠（樱花粉）：只在部分植被上出现，给岛一点季节感 */
    blossom: number
  }
  motion: {
    cloudSpeed: number
    windSway: number
    /** 活动脉动频率（Hz） */
    pulseHz: number
    /** 数据货物流速（沿路） */
    cargoSpeed: number
  }
  quality: Record<IslandQualityTier, IslandQualityBudget>
}

const HIGH: IslandQualityBudget = {
  shadows: true,
  antialias: true,
  maxPixelRatio: 2,
  props: 1,
  particles: 56,
  ornaments: 24,
  lights: 6,
  lightGain: 10,
  stars: 260,
  starSize: 2.6,
  powerPreference: 'high-performance',
  waterSegments: 64,
  targetFps: 60,
  terrainDetail: 72,
}

/**
 * 中档（WebGL2 可用但硬件偏低）：保留投影与灯火层次，砍掉最贵的三项 ——
 * 抗锯齿（MSAA 全屏 ×4 采样）、像素比（2 → 1.5，片元数少 44%）、装饰/粒子密度。
 * targetFps 取 45：中端机常配集显，60 会让风扇起飞，30 又明显顿；3/4 帧是折中。
 */
const MEDIUM: IslandQualityBudget = {
  shadows: true,
  antialias: false,
  maxPixelRatio: 1.5,
  props: 0.78,
  particles: 34,
  ornaments: 16,
  lights: 4,
  lightGain: 8,
  stars: 180,
  starSize: 2.4,
  powerPreference: 'low-power',
  waterSegments: 48,
  targetFps: 45,
  terrainDetail: 56,
}

const LOW: IslandQualityBudget = {
  shadows: false,
  antialias: false,
  maxPixelRatio: 1,
  props: 0.42,
  particles: 16,
  ornaments: 8,
  lights: 2,
  lightGain: 6,
  stars: 110,
  starSize: 2,
  powerPreference: 'low-power',
  waterSegments: 32,
  targetFps: 30,
  terrainDetail: 40,
}

/** 翠屿（默认）：温润的日式低多边形海岛 */
export const VERDANT_ISLAND_SPEC: IslandModelSpec = {
  id: 'verdant',
  label: '翠屿',
  terrain: { radius: 11, height: 0.6, jitter: 0.055, beachWidth: 2.0, rockBase: 1.7 },
  water: { level: -0.68, radius: 52, waveHeight: 0.08, waveLength: 1.8, waveSpeed: 0.85, opacity: 0.9 },
  layout: { coreRadius: 5, ringGap: 3.1, plotGap: 1.12, pierLength: 9, beaconRadius: 9.9, athenaeumRadius: 9.6, quayAngle: 0.62 },
  buildings: {
    core: {
      width: 4.4, depth: 4.4, height: 3.0, floors: 3, roof: 'dome', roofHeight: 1.7,
      wallColor: 0xe9e2d3, roofColor: 0x5d6fb8, trimColor: 0xb5793f, glowColor: 0xffd479,
      windowRows: 3, maxLights: 2,
    },
    workshop: {
      width: 2.7, depth: 2.3, height: 1.7, floors: 1, roof: 'gable', roofHeight: 0.95,
      wallColor: 0xeadcc0, roofColor: 0xc75b4e, trimColor: 0xb5793f, glowColor: 0xffc46b,
      windowRows: 1, maxLights: 1,
    },
    outpost: {
      width: 2.0, depth: 2.0, height: 1.4, floors: 1, roof: 'cone', roofHeight: 1.15,
      wallColor: 0xdacdb1, roofColor: 0x6fa2a8, trimColor: 0x8a6a44, glowColor: 0xffc46b,
      windowRows: 0, maxLights: 0,
    },
    depot: {
      width: 3.1, depth: 2.5, height: 1.25, floors: 1, roof: 'flat', roofHeight: 0.28,
      wallColor: 0xdcc9a3, roofColor: 0x8b6a4a, trimColor: 0x8a6a44, glowColor: 0xffc46b,
      windowRows: 0, maxLights: 0,
    },
    beacon: {
      width: 1.9, depth: 1.9, height: 4.2, floors: 4, roof: 'beacon', roofHeight: 1.3,
      wallColor: 0xf3f0e7, roofColor: 0xc9544b, trimColor: 0x9aa3ad, glowColor: 0xfff1a8,
      windowRows: 4, maxLights: 1,
    },
    pier: {
      width: 1.9, depth: 9, height: 0.38, floors: 1, roof: 'flat', roofHeight: 0.22,
      wallColor: 0xc39a68, roofColor: 0x8b6a4a, trimColor: 0x7d5027, glowColor: 0xffc46b,
      windowRows: 0, maxLights: 1,
    },
    athenaeum: {
      // 记忆塔：细高的层叠塔身 + 书页状鳍片 + 塔顶旋转光晕环（冷色记忆辉光）
      width: 2.6, depth: 2.6, height: 4.6, floors: 4, roof: 'dome', roofHeight: 1.3,
      wallColor: 0xe8e2f2, roofColor: 0x6b5fb5, trimColor: 0xc7a24b, glowColor: 0xb9d8ff,
      windowRows: 4, maxLights: 1,
    },
    construction: {
      // 施工地块：真实几何由 ConstructionPlotConfig（footprint/floorHeight/totalFloors）
      // 与工序快照逐部件增删，这里只是满足 Record<IslandEntityKind, …> 的占位模板，
      // 渲染器遇到 kind==='construction' 时走 constructionPlotBuilders，不读本模板。
      width: 2.4, depth: 2.4, height: 2.4, floors: 3, roof: 'flat', roofHeight: 0.2,
      wallColor: 0xb9b3a8, roofColor: 0x8a6a44, trimColor: 0xd9a441, glowColor: 0xe8873a,
      windowRows: 0, maxLights: 0,
    },
  },
  props: { trees: 26, rocks: 14, reeds: 10, clouds: 7 },
  lighting: {
    dayLength: 96, sunRadius: 24, sunHeight: 17,
    dayColor: 0xfff4dc, duskColor: 0xff9d6b, nightColor: 0x94b6ff, moonColor: 0xd7e3ff,
    ambientDay: 0.86, ambientNight: 0.34, fillIntensity: 0.62,
    lanternColor: 0xffc87a, lanternIntensity: 5.4,
    fogNear: 28, fogFar: 84, beamColor: 0xfff2b0,
  },
  palette: {
    grass: 0x7fc86a, grassDark: 0x5aa851, sand: 0xe9d8a6, rock: 0x8d8880,
    water: 0x3ea6d6, waterShallow: 0x93dcdc, waterDeep: 0x1d6ba4, wood: 0xb5793f, woodDark: 0x7d5027,
    stone: 0xcdc4b8, cloth: 0xf3f1e7, cloud: 0xffffff, blossom: 0xf6c6d8,
  },
  motion: { cloudSpeed: 0.34, windSway: 0.6, pulseHz: 0.55, cargoSpeed: 0.3 },
  quality: { high: HIGH, medium: MEDIUM, low: LOW },
}

/** 暮岛：暖色调干旱岛（热插拔皮肤示例：只换数据，不动代码） */
export const EMBER_ISLAND_SPEC: IslandModelSpec = {
  ...VERDANT_ISLAND_SPEC,
  id: 'ember',
  label: '暮岛',
  terrain: { radius: 11.4, height: 0.66, jitter: 0.07, beachWidth: 2.3, rockBase: 2.0 },
  layout: { coreRadius: 5.2, ringGap: 3.2, plotGap: 1.16, pierLength: 9.4, beaconRadius: 10.2, athenaeumRadius: 9.9, quayAngle: 0.72 },
  lighting: {
    ...VERDANT_ISLAND_SPEC.lighting,
    dayLength: 124, dayColor: 0xffe3b4, duskColor: 0xff8a4a, nightColor: 0x82a4e6,
    ambientDay: 0.9, ambientNight: 0.4, fillIntensity: 0.68, lanternIntensity: 6.0, fogFar: 96,
  },
  palette: {
    ...VERDANT_ISLAND_SPEC.palette,
    grass: 0xd8b26a, grassDark: 0xb98f4c, sand: 0xf0dcb4, rock: 0x8a7d70,
    water: 0x2f93bd, waterShallow: 0x8fd2c4, waterDeep: 0x18618f, wood: 0xa86f3c, woodDark: 0x6f4720,
    blossom: 0xf2b48c,
  },
}

export const ISLAND_SPECS: Record<string, IslandModelSpec> = {
  verdant: VERDANT_ISLAND_SPEC,
  ember: EMBER_ISLAND_SPEC,
}

export const DEFAULT_ISLAND_SPEC = VERDANT_ISLAND_SPEC

export function getIslandSpec(id?: string): IslandModelSpec {
  if (id && ISLAND_SPECS[id]) return ISLAND_SPECS[id]
  return DEFAULT_ISLAND_SPEC
}

export function listIslandSpecs(): { id: string; label: string }[] {
  return Object.values(ISLAND_SPECS).map((s) => ({ id: s.id, label: s.label }))
}

/** 记忆塔装饰细节（由质量预算推导，非渲染层常量） */
export interface IslandAthenaeumDetail {
  /** 塔顶光环上悬停的记忆光点数量 */
  motes: number
  /** 塔身书页状鳍片数量 */
  pages: number
  /** 塔顶同心光晕环数量 */
  rings: number
}

/**
 * 质量档 → 记忆塔细节数量。纯函数：几何构建器按此裁剪粒子与细节，
 * 于是 LOW 档自动少光点/少鳍片/少环（读预算，不写死魔法数）。
 */
export function athenaeumDetail(budget: IslandQualityBudget): IslandAthenaeumDetail {
  return {
    motes: Math.max(3, Math.round(budget.ornaments / 2)),
    pages: Math.max(2, Math.round(budget.ornaments / 4)),
    rings: Math.max(1, Math.round(budget.ornaments / 12)),
  }
}

function isColor(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 0xffffff
}

/**
 * 纯校验：返回问题列表（空数组 = 合法）。渲染器构造前必须调用，
 * 任何一条不合法都拒绝渲染（与桌宠 PetSvg 的诚实渲染口径一致）。
 */
export function validateIslandSpec(spec: IslandModelSpec): string[] {
  const errs: string[] = []
  if (!spec.id.trim()) errs.push('id 不能为空')
  if (!spec.label.trim()) errs.push('label 不能为空')

  const t = spec.terrain
  if (!(t.radius > 0)) errs.push('terrain.radius 必须 > 0')
  if (!(t.height > 0)) errs.push('terrain.height 必须 > 0')
  if (t.jitter < 0 || t.jitter > 1) errs.push('terrain.jitter 必须在 0..1')
  if (!(t.beachWidth > 0)) errs.push('terrain.beachWidth 必须 > 0')
  if (!(t.rockBase > 0)) errs.push('terrain.rockBase 必须 > 0')

  const w = spec.water
  if (!(w.waveLength > 0)) errs.push('water.waveLength 必须 > 0')
  if (!(w.waveSpeed > 0)) errs.push('water.waveSpeed 必须 > 0')
  if (w.opacity <= 0 || w.opacity > 1) errs.push('water.opacity 必须在 (0,1]')
  if (!(w.level < 0)) errs.push('water.level 必须低于草地平面（< 0）')
  if (!(w.radius > t.radius + t.beachWidth)) errs.push('water.radius 必须大于岛屿外缘')

  const l = spec.layout
  if (!(l.coreRadius > 0)) errs.push('layout.coreRadius 必须 > 0')
  if (!(l.ringGap > 0)) errs.push('layout.ringGap 必须 > 0')
  if (!(l.plotGap >= 1)) errs.push('layout.plotGap 必须 >= 1')
  if (!(l.pierLength > 0)) errs.push('layout.pierLength 必须 > 0')
  // 诚实性：建筑环必须落在草地上（按半宽判定：中心点 + 半宽不得越出岸线）
  const outerRing = l.coreRadius + l.ringGap + spec.buildings.outpost.width / 2
  if (outerRing >= t.radius) {
    errs.push(`建筑环外径 ${outerRing.toFixed(2)} 超出草地半径 ${t.radius}（建筑会掉进海里）`)
  }
  if (l.beaconRadius + spec.buildings.beacon.width / 2 >= t.radius) {
    errs.push('beaconRadius 过大：灯塔会超出草地半径')
  }
  const athenaeum = spec.buildings.athenaeum
  if (athenaeum && l.athenaeumRadius + athenaeum.width / 2 >= t.radius) {
    errs.push('athenaeumRadius 过大：记忆塔会超出草地半径')
  }

  for (const kind of ISLAND_KINDS) {
    const b = spec.buildings[kind]
    if (!b) {
      errs.push(`buildings.${kind} 缺失`)
      continue
    }
    if (!(b.width > 0) || !(b.depth > 0) || !(b.height > 0)) errs.push(`buildings.${kind} 尺寸必须 > 0`)
    if (b.roofHeight < 0) errs.push(`buildings.${kind}.roofHeight 不能为负`)
    if (b.maxLights < 0) errs.push(`buildings.${kind}.maxLights 不能为负`)
    for (const key of ['wallColor', 'roofColor', 'trimColor', 'glowColor'] as const) {
      if (!isColor(b[key])) errs.push(`buildings.${kind}.${key} 不是合法颜色`)
    }
  }

  for (const [key, value] of Object.entries(spec.palette)) {
    if (!isColor(value)) errs.push(`palette.${key} 不是合法颜色`)
  }
  // 水体径向色阶的配色不变量：浅滩必须最亮、远海必须最暗，
  // 否则「近岸透亮、远海沉静」的层次会反转或塌成单色（水面占画面约 85%，很显眼）。
  const lum = (c: number) => ((c >> 16) & 0xff) * 0.2126 + ((c >> 8) & 0xff) * 0.7152 + (c & 0xff) * 0.0722
  if (!(lum(spec.palette.waterShallow) > lum(spec.palette.water))) {
    errs.push('palette.waterShallow 必须比 palette.water 亮（浅滩应透亮）')
  }
  if (!(lum(spec.palette.water) > lum(spec.palette.waterDeep))) {
    errs.push('palette.water 必须比 palette.waterDeep 亮（远海应最深）')
  }
  for (const [key, value] of Object.entries(spec.lighting)) {
    const isColorKey = /Color$/.test(key)
    if (isColorKey ? !isColor(value as number) : !Number.isFinite(value as number)) {
      errs.push(`lighting.${key} 非法`)
    }
  }
  if (!(spec.lighting.dayLength > 0)) errs.push('lighting.dayLength 必须 > 0')
  if (spec.lighting.fillIntensity < 0) errs.push('lighting.fillIntensity 不能为负')
  if (spec.lighting.fogFar <= spec.lighting.fogNear) errs.push('lighting.fogFar 必须大于 fogNear')

  for (const tier of ISLAND_QUALITY_TIERS) {
    const q = spec.quality[tier]
    if (!q) {
      errs.push(`quality.${tier} 缺失`)
      continue
    }
    if (!(q.maxPixelRatio > 0)) errs.push(`quality.${tier}.maxPixelRatio 必须 > 0`)
    if (q.props < 0 || q.props > 1) errs.push(`quality.${tier}.props 必须在 0..1`)
    if (!(q.waterSegments >= 8)) errs.push(`quality.${tier}.waterSegments 至少 8 段`)
    if (!(q.terrainDetail >= 8)) errs.push(`quality.${tier}.terrainDetail 至少 8 段`)
    if (!(q.targetFps > 0)) errs.push(`quality.${tier}.targetFps 必须 > 0`)
    if (!(q.stars >= 0) || !(q.starSize > 0) || !(q.lightGain > 0)) errs.push(`quality.${tier} 星空/灯火预算非法`)
    if (q.powerPreference !== 'low-power' && q.powerPreference !== 'high-performance') {
      errs.push(`quality.${tier}.powerPreference 非法`)
    }
    if (!(q.lights >= 0) || !(q.particles >= 0) || !(q.ornaments >= 0)) errs.push(`quality.${tier} 光源/粒子/装饰预算不能为负`)
  }
  // 档位单调性：高档的每一项预算都不得低于低档，否则「降档」会反而更重（档位名不副实）。
  // 皮肤是热插拔数据，坏档位必须在构造渲染器前被拒，而不是等到中端机掉帧才被发现。
  for (let i = 0; i + 1 < ISLAND_QUALITY_TIERS.length; i++) {
    const heavier = ISLAND_QUALITY_TIERS[i]
    const lighter = ISLAND_QUALITY_TIERS[i + 1]
    const a = spec.quality[heavier]
    const b = spec.quality[lighter]
    if (!a || !b) continue
    for (const key of ['maxPixelRatio', 'props', 'particles', 'ornaments', 'lights', 'lightGain', 'stars', 'starSize', 'waterSegments', 'targetFps', 'terrainDetail'] as const) {
      if (a[key] < b[key]) errs.push(`quality.${heavier}.${key} 不得低于 quality.${lighter}.${key}（档位单调性）`)
    }
  }

  for (const [key, value] of Object.entries(spec.props)) {
    if (!Number.isInteger(value) || value < 0) errs.push(`props.${key} 必须是非负整数`)
  }
  for (const [key, value] of Object.entries(spec.motion)) {
    if (!(value >= 0)) errs.push(`motion.${key} 不能为负`)
  }

  return errs
}