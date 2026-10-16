/**
 * 岛屿几何构建器 —— 只负责「把 Spec + 类型 → Three.js 网格」，不含动画与交互。
 *
 * - **数据驱动**：所有尺寸/配色来自 IslandModelSpec（含 per-kind 建筑模板），本文件不写死数值。
 * - **材质复用**：静态件走共享材质缓存（墙面/屋顶/地形/道具），发光件按建筑 clone 一份
 *   （这样每座建筑的灯笼亮度可以独立变化，而不影响同类建筑与程序缓存）。
 * - **可动画标记**：构建结果通过 userData 暴露可动件（plank 铺板进度、spinner 旋转件、
 *   beam 灯塔光束、smoke 烟柱、glow 发光材质），渲染器只按标记驱动，不再反查几何结构。
 */

import * as THREE from 'three'
import type { IslandEntityKind, IslandWorkshopFlavor } from './islandContract'
import { athenaeumDetail, type IslandBuildingTemplate, type IslandModelSpec, type IslandQualityBudget } from './islandSpec'
import { clamp01, hashUnit, mixColor, waterColorAt } from './islandSim'

/** 构建器写入 userData 的可动画标记 */
export interface IslandBuildTags {
  entityId?: string
  glow?: boolean
  spinner?: boolean
  /** 旋转件绕哪根轴自转（缺省 'x'；水平光环环用 'y'，否则会被转成竖立） */
  spinAxis?: 'x' | 'y'
  beam?: boolean
  smoke?: boolean
  /** 栈桥铺板序号（从岸边起） */
  plank?: number
  /** 水面以下构件（低质量档可剔除） */
  submerged?: boolean
}

export interface IslandMaterialSet {
  /** 不透明静态件（墙/屋顶/地形/道具） */
  solid(color: number): THREE.MeshToonMaterial
  /** 半透明件（水面/光束/旗帜） */
  translucent(color: number, opacity: number): THREE.MeshToonMaterial
  gradientMap: THREE.DataTexture
  dispose(): void
}

/** 三段渐变贴图（与桌宠同款卡通着色口径） */
function buildToonGradient(): THREE.DataTexture {
  const levels = [0x7a7a7a, 0xececec, 0xffffff]
  const data = new Uint8Array(3 * 4)
  levels.forEach((c, i) => {
    data[i * 4 + 0] = (c >> 16) & 0xff
    data[i * 4 + 1] = (c >> 8) & 0xff
    data[i * 4 + 2] = c & 0xff
    data[i * 4 + 3] = 0xff
  })
  const tex = new THREE.DataTexture(data, 3, 1, THREE.RGBAFormat)
  tex.minFilter = THREE.NearestFilter
  tex.magFilter = THREE.NearestFilter
  tex.generateMipmaps = false
  tex.needsUpdate = true
  return tex
}

export function createIslandMaterials(): IslandMaterialSet {
  const gradientMap = buildToonGradient()
  const solids = new Map<string, THREE.MeshToonMaterial>()
  const translucents = new Map<string, THREE.MeshToonMaterial>()

  const set: IslandMaterialSet = {
    gradientMap,
    solid(color) {
      const key = `${color}`
      const hit = solids.get(key)
      if (hit) return hit
      const material = new THREE.MeshToonMaterial({ color, gradientMap })
      solids.set(key, material)
      return material
    },
    translucent(color, opacity) {
      const key = `${color}|${opacity}`
      const hit = translucents.get(key)
      if (hit) return hit
      const material = new THREE.MeshToonMaterial({
        color,
        gradientMap,
        transparent: true,
        opacity,
        side: THREE.DoubleSide,
        depthWrite: false,
      })
      translucents.set(key, material)
      return material
    },
    dispose() {
      for (const material of solids.values()) material.dispose()
      for (const material of translucents.values()) material.dispose()
      solids.clear()
      translucents.clear()
      gradientMap.dispose()
    },
  }
  return set
}

/** 发光件材质：每座建筑独立 clone，让灯笼亮度可以单独驱动 */
function glowMaterial(materials: IslandMaterialSet, color: number): THREE.MeshToonMaterial {
  const material = materials.solid(color).clone()
  material.emissive = new THREE.Color(color)
  material.emissiveIntensity = 0
  return material
}

function tag(object: THREE.Object3D, tags: IslandBuildTags): void {
  object.userData = { ...(object.userData as object), ...tags }
}

/** 岸线确定性抖动：按角度分桶 → 相邻顶点共享同一偏移，岸线连续而非锯齿乱跳 */
function jitterRim(geometry: THREE.BufferGeometry, amplitude: number, salt: number, radius: number): void {
  const position = geometry.attributes.position as THREE.BufferAttribute
  for (let i = 0; i < position.count; i++) {
    const x = position.getX(i)
    const z = position.getZ(i)
    const r = Math.hypot(x, z)
    if (r < radius * 0.2) continue
    const angle = Math.atan2(z, x)
    const bucket = Math.round(((angle + Math.PI) / (Math.PI * 2)) * 48)
    const n = hashUnit(bucket * 31 + salt) - 0.5
    const scale = 1 + n * amplitude
    position.setX(i, x * scale)
    position.setZ(i, z * scale)
    position.setY(i, position.getY(i) + (hashUnit(bucket * 17 + salt) - 0.5) * amplitude * 0.5)
  }
  geometry.computeVertexNormals()
}

/* ─────────────────────────── 地形 ─────────────────────────── */

export function buildTerrain(spec: IslandModelSpec, materials: IslandMaterialSet, detail: number): THREE.Group {
  const ground = new THREE.Group()
  ground.name = 'island-terrain'
  const T = spec.terrain
  const P = spec.palette

  // 草地台地（顶面 y=0）：顶点色做「高处亮、低处深」的渐层，避免一整块平板绿
  const grassGeo = new THREE.CylinderGeometry(T.radius, T.radius * 0.97, T.height, detail, 1)
  jitterRim(grassGeo, T.jitter, 1337, T.radius)
  const grassPosition = grassGeo.attributes.position as THREE.BufferAttribute
  const grassColors = new Float32Array(grassPosition.count * 3)
  const grassLow = mixColor(P.grassDark, P.rock, 0.12)
  const grassHigh = mixColor(P.grass, P.cloud, 0.18)
  // ⚠️ 顶点色必须写**线性**分量（与水面同一口径）：three 不对顶点色做 sRGB 解码，
  // 直接写调色板 sRGB 字节会让整片草地泛白发灰（实测草地渲成 #c6e9b5，而调色板是 #7fc86a）。
  // 草地是全岛唯一用顶点色的部件，沙滩/岩体走材质色（three 自动 sRGB→线性），
  // 口径不一致会让草地相对沙岸「发飘」——统一到线性后草地才落回调色板的绿。
  const grassScratch = new THREE.Color()
  for (let i = 0; i < grassPosition.count; i++) {
    const t = clamp01((grassPosition.getY(i) + T.height) / T.height)
    grassScratch.setHex(mixColor(grassLow, grassHigh, Math.pow(t, 0.65)), THREE.SRGBColorSpace)
    grassColors[i * 3] = grassScratch.r
    grassColors[i * 3 + 1] = grassScratch.g
    grassColors[i * 3 + 2] = grassScratch.b
  }
  grassGeo.setAttribute('color', new THREE.BufferAttribute(grassColors, 3))
  const grassMaterial = materials.solid(0xffffff).clone()
  grassMaterial.vertexColors = true
  const grass = new THREE.Mesh(grassGeo, grassMaterial)
  grass.position.y = -T.height / 2
  grass.castShadow = true
  grass.receiveShadow = true
  ground.add(grass)

  // 沙滩环：只露出水面一小截，其余没入浅水（大片沙架透过半透明水面会形成生硬色块）
  const beachHeight = T.height * 0.42
  const beachGeo = new THREE.CylinderGeometry(
    T.radius + T.beachWidth,
    T.radius + T.beachWidth * 0.82,
    beachHeight,
    detail,
    1,
  )
  jitterRim(beachGeo, T.jitter * 0.8, 7331, T.radius)
  const beach = new THREE.Mesh(beachGeo, materials.solid(P.sand))
  beach.position.y = -beachHeight / 2 - 0.55
  beach.receiveShadow = true
  ground.add(beach)

  // 岛底岩体 + 尖端（整体压到水面以下，避免深色岩壁露出水面形成一圈黑边）
  const rockGeo = new THREE.CylinderGeometry(T.radius * 0.94, T.radius * 0.42, T.rockBase, detail, 1)
  const rock = new THREE.Mesh(rockGeo, materials.solid(P.rock))
  rock.position.y = -T.height - T.rockBase / 2 - 0.16
  ground.add(rock)
  const tip = new THREE.Mesh(new THREE.ConeGeometry(T.radius * 0.42, T.rockBase * 0.8, Math.max(6, detail / 3)), materials.solid(P.rock))
  tip.position.y = -T.height - T.rockBase - T.rockBase * 0.4 - 0.16
  tip.rotation.x = Math.PI
  ground.add(tip)

  // 缓坡丘陵（确定性摆放，纯装饰）
  const hillGeo = new THREE.SphereGeometry(1, 14, 10)
  const hillCount = Math.max(3, Math.round(detail / 14))
  for (let i = 0; i < hillCount; i++) {
    const angle = (i / hillCount) * Math.PI * 2 + 0.7
    const radius = T.radius * (0.48 + hashUnit(i * 13 + 29) * 0.22)
    const hill = new THREE.Mesh(hillGeo, materials.solid(P.grassDark))
    hill.scale.set(
      T.radius * (0.16 + hashUnit(i * 7) * 0.1),
      T.height * (0.7 + hashUnit(i * 11) * 0.5),
      T.radius * (0.12 + hashUnit(i * 5) * 0.08),
    )
    hill.position.set(Math.cos(angle) * radius, -T.height * 0.42, Math.sin(angle) * radius)
    hill.castShadow = true
    ground.add(hill)
  }
  return ground
}

/* ─────────────────────────── 水面 ─────────────────────────── */

export interface IslandWaterHandle {
  mesh: THREE.Mesh
  /** 基准顶点（每帧在其上叠加波高，避免误差累积） */
  basePositions: Float32Array
  material: THREE.MeshStandardMaterial
  /** 浅滩浪花带（透明度随潮汐脉动） */
  foam: THREE.Mesh
  foamMaterial: THREE.MeshBasicMaterial
}

export function buildWater(spec: IslandModelSpec, segments: number): IslandWaterHandle {
  const W = spec.water
  const inner = spec.terrain.radius * 0.86
  // 径向分段要足够承载色阶：段数太少会让「浅滩→远海」出现肉眼可见的同心色环
  const RADIAL_SEGMENTS = 12
  const geometry = new THREE.RingGeometry(inner, W.radius, segments, RADIAL_SEGMENTS)
  geometry.rotateX(-Math.PI / 2)
  const position = geometry.attributes.position as THREE.BufferAttribute
  // 逐顶点写入径向色阶（浅滩青绿 → 主体海色 → 远海靛蓝，见 waterColorAt）。
  // 材质基色保持白色，把「水色」这件事完全交给顶点色 + 昼夜染色（Island3D.applySky），
  // 否则基色与顶点色相乘会把色阶压暗两次。
  //
  // ⚠️ 顶点色必须写**线性**分量：three 不对顶点色做 sRGB 解码，直接写调色板的 sRGB 字节
  // 会让整片海发灰发白（调色板 #3ea6d6 会被当成线性值再经 sRGB 编码输出，亮一大截）。
  // 用 `Color.setHex(hex, SRGBColorSpace)` 取工作空间（线性）分量即可与材质色口径一致。
  const colors = new Float32Array(position.count * 3)
  const scratch = new THREE.Color()
  const span = Math.max(1e-6, W.radius - inner)
  for (let i = 0; i < position.count; i++) {
    const r = Math.hypot(position.getX(i), position.getZ(i))
    scratch.setHex(waterColorAt(spec, (r - inner) / span), THREE.SRGBColorSpace)
    colors[i * 3] = scratch.r
    colors[i * 3 + 1] = scratch.g
    colors[i * 3 + 2] = scratch.b
  }
  geometry.setAttribute('color', new THREE.BufferAttribute(colors, 3))
  const material = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    vertexColors: true,
    roughness: 0.18,
    metalness: 0.24,
    transparent: true,
    opacity: W.opacity,
  })
  const mesh = new THREE.Mesh(geometry, material)
  mesh.position.y = W.level
  mesh.receiveShadow = false
  const foamMaterial = foamMaterialOf(spec)
  return {
    mesh,
    basePositions: Float32Array.from(position.array as ArrayLike<number>),
    material,
    foam: buildFoam(spec, segments, foamMaterial),
    foamMaterial,
  }
}

function foamMaterialOf(spec: IslandModelSpec): THREE.MeshBasicMaterial {
  return new THREE.MeshBasicMaterial({
    // 浅滩浪花：白云色与海水的中间调，比海水亮、比纯白柔
    color: mixColor(spec.palette.cloud, spec.palette.water, 0.42),
    transparent: true,
    opacity: 0.26,
    depthWrite: false,
  })
}

/** 岸线浪花带：贴着沙滩外缘的一圈亮色浅水，是《我们的岛》海面质感的关键 */
function buildFoam(spec: IslandModelSpec, segments: number, material: THREE.MeshBasicMaterial): THREE.Mesh {
  const T = spec.terrain
  // 起点必须落在「岸线最大抖动半径」之外：否则浪花带会被不平整的沙滩切成交替断续的色块
  const inner = T.radius + T.beachWidth * 1.18
  const geometry = new THREE.RingGeometry(inner, inner * 1.14, segments, 1)
  geometry.rotateX(-Math.PI / 2)
  const mesh = new THREE.Mesh(geometry, material)
  // 必须高于「波峰」（water.level + waveHeight）：否则会被逐帧起伏的水面切成断续的楔形色块
  mesh.position.y = spec.water.level + spec.water.waveHeight + 0.03
  mesh.renderOrder = 2
  return mesh
}

/* ─────────────────────────── 装饰道具（实例化） ─────────────────────────── */

export interface IslandKeepOut {
  x: number
  z: number
  radius: number
}

export function buildProps(
  spec: IslandModelSpec,
  materials: IslandMaterialSet,
  quality: number,
  keepOut: readonly IslandKeepOut[],
): THREE.Group {
  const group = new THREE.Group()
  group.name = 'island-props'
  const T = spec.terrain
  const P = spec.palette

  const fits = (x: number, z: number, pad: number) => {
    if (Math.hypot(x, z) > T.radius * 0.92) return false
    return keepOut.every((zone) => Math.hypot(x - zone.x, z - zone.z) > zone.radius + pad)
  }
  // 确定性采样：环带 + 角度散列，跳过被建筑占据的位置
  const samples = (count: number, rMin: number, rMax: number, salt: number, pad: number) => {
    const out: { x: number; z: number; scale: number }[] = []
    const wanted = Math.round(count * quality)
    for (let i = 0; out.length < wanted && i < wanted * 6; i++) {
      const angle = hashUnit(i * 3 + salt) * Math.PI * 2
      const radius = rMin + hashUnit(i * 5 + salt + 1) * (rMax - rMin)
      const x = Math.cos(angle) * radius
      const z = Math.sin(angle) * radius
      if (!fits(x, z, pad)) continue
      out.push({ x, z, scale: 0.75 + hashUnit(i * 7 + salt + 2) * 0.6 })
    }
    return out
  }

  const dummy = new THREE.Object3D()

  // 树：树干 + 双层树冠（下深上亮），约三成开成花树 —— 让植被有季节层次
  const trees = samples(spec.props.trees, T.radius * 0.34, T.radius * 0.9, 101, 1.5)
  if (trees.length > 0) {
    const trunk = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.11, 0.16, 0.9, 6), materials.solid(P.woodDark), trees.length)
    const lower = new THREE.InstancedMesh(new THREE.ConeGeometry(0.66, 1.0, 8), materials.solid(P.grassDark), trees.length)
    const upper = new THREE.InstancedMesh(
      new THREE.ConeGeometry(0.46, 0.88, 8),
      materials.solid(mixColor(P.grass, P.cloud, 0.24)),
      trees.length,
    )
    const blossomSlots = trees.map((_, i) => i).filter((i) => hashUnit(i * 29 + 7) < 0.3)
    const blossom = blossomSlots.length > 0
      ? new THREE.InstancedMesh(new THREE.ConeGeometry(0.4, 0.72, 8), materials.solid(P.blossom), blossomSlots.length)
      : null
    let blossomIndex = 0
    trees.forEach((tree, i) => {
      dummy.position.set(tree.x, 0.45 * tree.scale, tree.z)
      dummy.scale.setScalar(tree.scale)
      dummy.rotation.set(0, hashUnit(i * 9 + 7) * Math.PI, 0)
      dummy.updateMatrix()
      trunk.setMatrixAt(i, dummy.matrix)
      dummy.position.y = 0.4 * tree.scale
      dummy.updateMatrix()
      lower.setMatrixAt(i, dummy.matrix)
      dummy.position.y = 1.02 * tree.scale
      dummy.updateMatrix()
      upper.setMatrixAt(i, dummy.matrix)
      if (blossom && blossomSlots.includes(i)) {
        dummy.position.y = 1.42 * tree.scale
        dummy.updateMatrix()
        blossom.setMatrixAt(blossomIndex++, dummy.matrix)
      }
    })
    trunk.castShadow = true
    lower.castShadow = true
    trunk.instanceMatrix.needsUpdate = true
    lower.instanceMatrix.needsUpdate = true
    upper.instanceMatrix.needsUpdate = true
    group.add(trunk, lower, upper)
    if (blossom) {
      blossom.castShadow = true
      blossom.instanceMatrix.needsUpdate = true
      group.add(blossom)
    }
  }

  // 礁石
  const rocks = samples(spec.props.rocks, T.radius * 0.7, T.radius * 1.02, 211, 0.9)
  if (rocks.length > 0) {
    const mesh = new THREE.InstancedMesh(new THREE.IcosahedronGeometry(0.42, 0), materials.solid(P.rock), rocks.length)
    rocks.forEach((rock, i) => {
      dummy.position.set(rock.x, -0.1, rock.z)
      dummy.scale.set(rock.scale, rock.scale * (0.5 + hashUnit(i) * 0.5), rock.scale)
      dummy.rotation.set(hashUnit(i * 3) * 0.6, hashUnit(i * 5) * Math.PI, hashUnit(i * 7) * 0.6)
      dummy.updateMatrix()
      mesh.setMatrixAt(i, dummy.matrix)
    })
    mesh.castShadow = true
    mesh.instanceMatrix.needsUpdate = true
    group.add(mesh)
  }

  // 芦苇（岸边细高）
  const reeds = samples(spec.props.reeds, T.radius * 0.86, T.radius * 1.2, 307, 0.4)
  if (reeds.length > 0) {
    const mesh = new THREE.InstancedMesh(new THREE.ConeGeometry(0.07, 0.8, 4), materials.solid(P.grassDark), reeds.length)
    reeds.forEach((reed, i) => {
      dummy.position.set(reed.x, 0.28, reed.z)
      dummy.scale.set(1, reed.scale * 1.4, 1)
      dummy.rotation.set(hashUnit(i * 11) * 0.3, 0, hashUnit(i * 13) * 0.3)
      dummy.updateMatrix()
      mesh.setMatrixAt(i, dummy.matrix)
    })
    mesh.instanceMatrix.needsUpdate = true
    group.add(mesh)
  }
  return group
}

/** 云（独立网格：会随风漂移，故不实例化）；材质随昼夜回传，便于夜间压暗 */
export function buildClouds(spec: IslandModelSpec, quality: number): { group: THREE.Group; material: THREE.MeshBasicMaterial } {
  const group = new THREE.Group()
  group.name = 'island-clouds'
  const material = new THREE.MeshBasicMaterial({
    color: spec.palette.cloud,
    transparent: true,
    opacity: 0.72,
    // 云色已按昼夜显式驱动，再叠雾会让夜云被雾色染亮
    fog: false,
  })
  const count = Math.max(2, Math.round(spec.props.clouds * quality))
  const geometry = new THREE.SphereGeometry(1, 8, 6)
  for (let i = 0; i < count; i++) {
    const cloud = new THREE.Group()
    const lobes = 3 + Math.round(hashUnit(i * 3) * 2)
    for (let j = 0; j < lobes; j++) {
      const lobe = new THREE.Mesh(geometry, material)
      lobe.position.set((j - lobes / 2) * 1.1, hashUnit(i * 7 + j) * 0.4, hashUnit(i * 9 + j) * 0.5)
      lobe.scale.set(1.5 + hashUnit(i + j) * 0.9, 0.85 + hashUnit(i * 5 + j) * 0.4, 1.1)
      cloud.add(lobe)
    }
    const angle = hashUnit(i * 13) * Math.PI * 2
    const radius = spec.terrain.radius * (1.6 + hashUnit(i * 17) * 1.6)
    cloud.position.set(Math.cos(angle) * radius, spec.terrain.radius * (0.62 + hashUnit(i * 19) * 0.4), Math.sin(angle) * radius)
    cloud.scale.setScalar(1.2 + hashUnit(i * 23) * 0.8)
    group.add(cloud)
  }
  return { group, material }
}

/* ─────────────────────────── 建筑 ─────────────────────────── */

/** 水面建筑（码头/仓库）的桩柱 */
function addPiles(parent: THREE.Group, template: IslandBuildingTemplate, materials: IslandMaterialSet, spec: IslandModelSpec, submerged = true): void {
  const pileGeo = new THREE.CylinderGeometry(0.09, 0.09, 1.5, 6)
  const material = materials.solid(spec.palette.woodDark)
  const half = template.width / 2 - 0.24
  for (const dx of [-half, half]) {
    for (let i = 0; i < 3; i++) {
      const z = -template.depth / 2 + 0.5 + i * ((template.depth - 1) / 2 || 1)
      const pile = new THREE.Mesh(pileGeo, material)
      pile.position.set(dx, -0.62, z)
      if (submerged) tag(pile, { submerged: true })
      parent.add(pile)
    }
  }
}

/** 双坡屋顶（两块斜板 + 屋脊） */
function addGableRoof(parent: THREE.Group, template: IslandBuildingTemplate, materials: IslandMaterialSet, y: number): void {
  const span = Math.max(template.width, template.depth) * 0.72
  const slope = Math.hypot(span / 2, template.roofHeight)
  const planeGeo = new THREE.BoxGeometry(slope, 0.14, template.depth * 1.12)
  const material = materials.solid(template.roofColor)
  const angle = Math.atan2(template.roofHeight, span / 2)
  for (const side of [-1, 1]) {
    const plane = new THREE.Mesh(planeGeo, material)
    plane.position.set((side * span) / 4, y + template.roofHeight / 2, 0)
    plane.rotation.z = side * angle
    plane.castShadow = true
    parent.add(plane)
  }
  const ridge = new THREE.Mesh(new THREE.BoxGeometry(0.16, 0.16, template.depth * 1.16), materials.solid(template.trimColor))
  ridge.position.y = y + template.roofHeight
  parent.add(ridge)
}

/** 发光件汇总（构建后交给渲染器按状态驱动亮度） */
export interface IslandBuildingHandle {
  group: THREE.Group
  glowMaterials: THREE.MeshToonMaterial[]
  /** 旋转件（水车等） */
  spinners: THREE.Object3D[]
  /** 灯塔光束（枢轴 Group：绕 Y 扫动，子节点为锥体） */
  beam?: THREE.Group
  /** 栈桥铺板（按进度显隐） */
  planks: THREE.Mesh[]
  /** 烟柱（活动时升起） */
  smoke: THREE.Object3D[]
}

export function buildBuilding(
  kind: IslandEntityKind,
  template: IslandBuildingTemplate,
  spec: IslandModelSpec,
  materials: IslandMaterialSet,
  flavor?: IslandWorkshopFlavor,
  budget: IslandQualityBudget = spec.quality.high,
): IslandBuildingHandle {
  const group = new THREE.Group()
  const glowMaterials: THREE.MeshToonMaterial[] = []
  const spinners: THREE.Object3D[] = []
  const planks: THREE.Mesh[] = []
  const smoke: THREE.Object3D[] = []
  let beam: THREE.Group | undefined
  const P = spec.palette

  switch (kind) {
    case 'core': {
      const floors = Math.max(1, template.floors)
      const floorHeight = template.height / floors
      for (let i = 0; i < floors; i++) {
        const radius = (template.width / 2) * (1 - i * 0.06)
        const floor = new THREE.Mesh(
          new THREE.CylinderGeometry(radius, radius * 1.04, floorHeight * 1.02, 12),
          materials.solid(i % 2 === 0 ? template.wallColor : P.stone),
        )
        floor.position.y = floorHeight * (i + 0.5)
        floor.castShadow = true
        floor.receiveShadow = true
        group.add(floor)
      }
      // 腰线
      for (let i = 1; i < floors; i++) {
        const band = new THREE.Mesh(
          new THREE.TorusGeometry((template.width / 2) * (1 - i * 0.06) + 0.02, 0.06, 6, 20),
          materials.solid(template.trimColor),
        )
        band.rotation.x = Math.PI / 2
        band.position.y = floorHeight * i
        group.add(band)
      }
      // 环形窗（每层一组，共用一个发光材质 → 该建筑的灯笼亮度整体联动）
      if (template.windowRows > 0) {
        const windowMaterial = glowMaterial(materials, template.glowColor)
        glowMaterials.push(windowMaterial)
        const windowGeo = new THREE.BoxGeometry(0.22, 0.3, 0.07)
        for (let row = 0; row < template.windowRows; row++) {
          const radius = (template.width / 2) * (1 - row * 0.06) + 0.01
          for (let i = 0; i < 6; i++) {
            const angle = (i / 6) * Math.PI * 2 + row * 0.35
            const pane = new THREE.Mesh(windowGeo, windowMaterial)
            pane.position.set(Math.cos(angle) * radius, floorHeight * (row + 0.55), Math.sin(angle) * radius)
            pane.rotation.y = -angle
            tag(pane, { glow: true })
            group.add(pane)
          }
        }
      }
      // 穹顶 + 尖顶 + 顶端灯笼
      const dome = new THREE.Mesh(
        new THREE.SphereGeometry(template.width / 2 + 0.12, 16, 10, 0, Math.PI * 2, 0, Math.PI / 2),
        materials.solid(template.roofColor),
      )
      dome.position.y = template.height
      dome.castShadow = true
      group.add(dome)
      const spire = new THREE.Mesh(new THREE.ConeGeometry(0.26, template.roofHeight, 8), materials.solid(template.trimColor))
      spire.position.y = template.height + template.width / 2 + template.roofHeight / 2 - 0.3
      spire.castShadow = true
      group.add(spire)
      const lanternMaterial = glowMaterial(materials, template.glowColor)
      glowMaterials.push(lanternMaterial)
      const lantern = new THREE.Mesh(new THREE.SphereGeometry(0.34, 12, 10), lanternMaterial)
      lantern.position.y = template.height + template.width / 2 + template.roofHeight - 0.4
      tag(lantern, { glow: true })
      group.add(lantern)
      break
    }

    case 'workshop': {
      const body = new THREE.Mesh(new THREE.BoxGeometry(template.width, template.height, template.depth), materials.solid(template.wallColor))
      body.position.y = template.height / 2
      body.castShadow = true
      body.receiveShadow = true
      group.add(body)
      // 石基
      const base = new THREE.Mesh(new THREE.BoxGeometry(template.width * 1.06, 0.18, template.depth * 1.06), materials.solid(P.stone))
      base.position.y = 0.09
      group.add(base)
      addGableRoof(group, template, materials, template.height)
      // 门 + 窗（正面 z+）
      const door = new THREE.Mesh(new THREE.BoxGeometry(0.42, 0.62, 0.06), materials.solid(template.trimColor))
      door.position.set(0, 0.36, template.depth / 2 + 0.01)
      group.add(door)
      if (template.windowRows > 0) {
        const material = glowMaterial(materials, template.glowColor)
        glowMaterials.push(material)
        const pane = new THREE.Mesh(new THREE.BoxGeometry(0.34, 0.32, 0.06), material)
        pane.position.set(template.width * 0.28, template.height * 0.62, template.depth / 2 + 0.01)
        tag(pane, { glow: true })
        group.add(pane)
      }
      // 形态附件（由工具名派生：熔炉/观象台/书库/作坊）
      if (flavor === 'forge') {
        const chimney = new THREE.Mesh(new THREE.BoxGeometry(0.3, template.height + template.roofHeight * 0.9, 0.3), materials.solid(P.rock))
        chimney.position.set(-template.width * 0.3, (template.height + template.roofHeight * 0.9) / 2, -template.depth * 0.22)
        chimney.castShadow = true
        group.add(chimney)
        const emberMaterial = glowMaterial(materials, 0xff8a3c)
        glowMaterials.push(emberMaterial)
        const ember = new THREE.Mesh(new THREE.SphereGeometry(0.16, 8, 6), emberMaterial)
        ember.position.set(-template.width * 0.3, template.height + template.roofHeight * 0.9 + 0.1, -template.depth * 0.22)
        tag(ember, { glow: true })
        group.add(ember)
      } else if (flavor === 'observatory') {
        const dome = new THREE.Mesh(
          new THREE.SphereGeometry(template.depth * 0.34, 12, 8, 0, Math.PI * 2, 0, Math.PI / 2),
          materials.solid(template.roofColor),
        )
        dome.position.y = template.height + template.roofHeight
        dome.castShadow = true
        group.add(dome)
        const scope = new THREE.Mesh(new THREE.CylinderGeometry(0.08, 0.1, 0.9, 8), materials.solid(P.stone))
        scope.position.set(0, template.height + template.roofHeight + 0.34, 0.1)
        scope.rotation.x = Math.PI / 3
        group.add(scope)
      } else if (flavor === 'library') {
        for (let i = 0; i < 3; i++) {
          const stack = new THREE.Mesh(
            new THREE.BoxGeometry(0.5 - i * 0.06, 0.16, 0.36 - i * 0.04),
            materials.solid(i % 2 === 0 ? template.trimColor : template.wallColor),
          )
          stack.position.set(template.width * 0.34, template.height + 0.1 + i * 0.18, -template.depth * 0.1)
          stack.rotation.y = i * 0.4
          group.add(stack)
        }
      } else {
        // mill：侧挂水车（旋转件）
        const wheel = new THREE.Group()
        const ring = new THREE.Mesh(new THREE.TorusGeometry(0.44, 0.06, 6, 14), materials.solid(P.woodDark))
        wheel.add(ring)
        for (let i = 0; i < 6; i++) {
          const spoke = new THREE.Mesh(new THREE.BoxGeometry(0.86, 0.05, 0.05), materials.solid(P.wood))
          spoke.rotation.z = (i / 6) * Math.PI
          wheel.add(spoke)
        }
        wheel.position.set(-template.width / 2 - 0.16, 0.5, 0)
        wheel.rotation.y = Math.PI / 2
        tag(wheel, { spinner: true })
        spinners.push(wheel)
        group.add(wheel)
      }
      break
    }

    case 'outpost': {
      const body = new THREE.Mesh(new THREE.CylinderGeometry(template.width * 0.42, template.width * 0.5, template.height, 7), materials.solid(template.wallColor))
      body.position.y = template.height / 2
      body.castShadow = true
      group.add(body)
      const roof = new THREE.Mesh(new THREE.ConeGeometry(template.width * 0.62, template.roofHeight, 7), materials.solid(template.roofColor))
      roof.position.y = template.height + template.roofHeight / 2
      roof.castShadow = true
      group.add(roof)
      // 旗杆 + 旗（活动时飘动）
      const pole = new THREE.Mesh(new THREE.CylinderGeometry(0.04, 0.04, 1.1, 5), materials.solid(P.woodDark))
      pole.position.set(0, template.height + template.roofHeight + 0.45, 0)
      group.add(pole)
      const flag = new THREE.Mesh(new THREE.PlaneGeometry(0.5, 0.3), materials.translucent(spec.palette.cloth, 0.92))
      flag.position.set(0.27, template.height + template.roofHeight + 0.75, 0)
      flag.rotation.y = -Math.PI / 2
      group.add(flag)
      break
    }

    case 'depot': {
      const body = new THREE.Mesh(new THREE.BoxGeometry(template.width, template.height, template.depth), materials.solid(template.wallColor))
      body.position.y = template.height / 2
      body.castShadow = true
      body.receiveShadow = true
      group.add(body)
      const roof = new THREE.Mesh(new THREE.BoxGeometry(template.width * 1.1, template.roofHeight, template.depth * 1.1), materials.solid(template.roofColor))
      roof.position.y = template.height + template.roofHeight / 2
      roof.castShadow = true
      group.add(roof)
      // 货箱（装载率驱动显示数量）
      for (let i = 0; i < 3; i++) {
        const crate = new THREE.Mesh(new THREE.BoxGeometry(0.42, 0.42, 0.42), materials.solid(i % 2 === 0 ? P.wood : P.woodDark))
        crate.position.set(-0.5 + i * 0.52, 0.21, template.depth / 2 + 0.36)
        crate.rotation.y = 0.2 * i
        crate.castShadow = true
        tag(crate, { plank: i })
        group.add(crate)
      }
      // 吊臂
      const arm = new THREE.Mesh(new THREE.BoxGeometry(0.16, 1.5, 0.16), materials.solid(P.woodDark))
      arm.position.set(template.width * 0.36, template.height + 0.5, 0)
      arm.rotation.z = -0.5
      group.add(arm)
      addPiles(group, template, materials, spec)
      break
    }

    case 'beacon': {
      const shaft = new THREE.Mesh(
        new THREE.CylinderGeometry(template.width * 0.3, template.width * 0.46, template.height, 10),
        materials.solid(template.wallColor),
      )
      shaft.position.y = template.height / 2
      shaft.castShadow = true
      group.add(shaft)
      for (let i = 0; i < template.windowRows; i++) {
        const band = new THREE.Mesh(
          new THREE.TorusGeometry(template.width * (0.3 + i * 0.03), 0.05, 5, 16),
          materials.solid(i % 2 === 0 ? template.roofColor : template.trimColor),
        )
        band.rotation.x = Math.PI / 2
        band.position.y = (template.height / (template.windowRows + 1)) * (i + 1)
        group.add(band)
      }
      const gallery = new THREE.Mesh(new THREE.TorusGeometry(template.width * 0.42, 0.06, 5, 16), materials.solid(template.trimColor))
      gallery.rotation.x = Math.PI / 2
      gallery.position.y = template.height
      group.add(gallery)
      const lampMaterial = glowMaterial(materials, template.glowColor)
      glowMaterials.push(lampMaterial)
      const lamp = new THREE.Mesh(new THREE.CylinderGeometry(template.width * 0.26, template.width * 0.26, template.roofHeight * 0.8, 10), lampMaterial)
      lamp.position.y = template.height + template.roofHeight * 0.42
      tag(lamp, { glow: true })
      group.add(lamp)
      const cap = new THREE.Mesh(new THREE.ConeGeometry(template.width * 0.38, template.roofHeight * 0.6, 10), materials.solid(template.roofColor))
      cap.position.y = template.height + template.roofHeight * 0.9
      cap.castShadow = true
      group.add(cap)
      // 旋转光束：枢轴绕 Y 扫动，锥体沿本地 +X 朝外张开（锥尖贴近灯室，宽端远离）
      const beamMaterial = materials.translucent(spec.lighting.beamColor, 0.16).clone()
      const beamLength = spec.terrain.radius * 1.5
      const beamPivot = new THREE.Group()
      beamPivot.position.y = template.height + template.roofHeight * 0.42
      const beamMesh = new THREE.Mesh(
        new THREE.ConeGeometry(1.5, beamLength, 12, 1, true),
        beamMaterial,
      )
      beamMesh.rotation.z = Math.PI / 2
      beamMesh.position.x = beamLength / 2
      tag(beamMesh, { beam: true })
      beamPivot.add(beamMesh)
      group.add(beamPivot)
      beam = beamPivot
      break
    }

    case 'pier': {
      const count = Math.max(3, Math.round(template.depth / 0.7))
      const plankGeo = new THREE.BoxGeometry(template.width, 0.12, 0.56)
      const plankMaterial = materials.solid(template.wallColor)
      for (let i = 0; i < count; i++) {
        const plank = new THREE.Mesh(plankGeo, plankMaterial)
        plank.position.set(0, 0.02, i * 0.7 + 0.36)
        plank.castShadow = true
        tag(plank, { plank: i })
        planks.push(plank)
        group.add(plank)
      }
      // 栏杆 + 桩柱
      const railMaterial = materials.solid(P.woodDark)
      for (const dx of [-template.width / 2 + 0.1, template.width / 2 - 0.1]) {
        const rail = new THREE.Mesh(new THREE.BoxGeometry(0.09, 0.09, template.depth), railMaterial)
        rail.position.set(dx, 0.44, template.depth / 2)
        group.add(rail)
        for (let i = 0; i < count; i += 2) {
          const post = new THREE.Mesh(new THREE.BoxGeometry(0.09, 0.5, 0.09), railMaterial)
          post.position.set(dx, 0.22, i * 0.7 + 0.36)
          group.add(post)
        }
      }
      for (let i = 0; i < count; i += 2) {
        for (const dx of [-template.width / 2 + 0.2, template.width / 2 - 0.2]) {
          const pile = new THREE.Mesh(new THREE.CylinderGeometry(0.1, 0.1, 1.6, 6), railMaterial)
          pile.position.set(dx, -0.75, i * 0.7 + 0.36)
          tag(pile, { submerged: true })
          group.add(pile)
        }
      }
      // 尽头灯笼柱
      const lampMaterial = glowMaterial(materials, template.glowColor)
      glowMaterials.push(lampMaterial)
      const post = new THREE.Mesh(new THREE.CylinderGeometry(0.06, 0.08, 1.5, 6), railMaterial)
      post.position.set(0, 0.75, template.depth)
      group.add(post)
      const lamp = new THREE.Mesh(new THREE.SphereGeometry(0.22, 10, 8), lampMaterial)
      lamp.position.set(0, 1.55, template.depth)
      tag(lamp, { glow: true })
      group.add(lamp)
      break
    }

    case 'athenaeum': {
      // 记忆塔：层叠收分的塔身（记忆是一层层叠起的书架）+ 书页鳍片 + 塔顶旋转光晕环。
      // 细节数量全部来自质量预算（athenaeumDetail），LOW 档自动少光点/少鳍片/少环。
      const detail = athenaeumDetail(budget)
      const floors = Math.max(1, template.floors)
      const floorHeight = template.height / floors
      const radiusAt = (i: number) => template.width * 0.5 * (1 - i * 0.07)
      for (let i = 0; i < floors; i++) {
        const body = new THREE.Mesh(
          new THREE.CylinderGeometry(radiusAt(i), radiusAt(i) * 1.06, floorHeight * 1.02, 8),
          materials.solid(i % 2 === 0 ? template.wallColor : P.stone),
        )
        body.position.y = floorHeight * (i + 0.5)
        body.castShadow = true
        body.receiveShadow = true
        group.add(body)
      }
      // 腰线（每层一圈细环）
      for (let i = 1; i < floors; i++) {
        const band = new THREE.Mesh(
          new THREE.TorusGeometry(radiusAt(i) + 0.02, 0.05, 5, 16),
          materials.solid(template.trimColor),
        )
        band.rotation.x = Math.PI / 2
        band.position.y = floorHeight * i
        group.add(band)
      }
      // 书页状鳍片：沿塔身竖立的薄板（方位由确定性散列给出，无随机）
      const pageMaterial = materials.solid(template.trimColor)
      for (let i = 0; i < detail.pages; i++) {
        const angle = (i / detail.pages) * Math.PI * 2 + hashUnit(i * 7 + 3) * 0.2
        const page = new THREE.Mesh(new THREE.BoxGeometry(0.06, template.height * 0.72, 0.42), pageMaterial)
        page.position.set(
          Math.cos(angle) * (template.width * 0.5 + 0.06),
          template.height * 0.44,
          Math.sin(angle) * (template.width * 0.5 + 0.06),
        )
        page.rotation.y = -angle
        group.add(page)
      }
      // 冷色记忆窗（每层一组，共用一个发光材质 → 整塔亮度联动）
      if (template.windowRows > 0) {
        const windowMaterial = glowMaterial(materials, template.glowColor)
        glowMaterials.push(windowMaterial)
        const windowGeo = new THREE.BoxGeometry(0.18, 0.26, 0.06)
        for (let row = 0; row < template.windowRows; row++) {
          const radius = radiusAt(row) + 0.01
          for (let i = 0; i < 4; i++) {
            const angle = (i / 4) * Math.PI * 2 + row * 0.5
            const pane = new THREE.Mesh(windowGeo, windowMaterial)
            pane.position.set(Math.cos(angle) * radius, floorHeight * (row + 0.55), Math.sin(angle) * radius)
            pane.rotation.y = -angle
            tag(pane, { glow: true })
            group.add(pane)
          }
        }
      }
      // 穹顶 + 尖顶
      const dome = new THREE.Mesh(
        new THREE.SphereGeometry(template.width * 0.5 + 0.06, 14, 9, 0, Math.PI * 2, 0, Math.PI / 2),
        materials.solid(template.roofColor),
      )
      dome.position.y = template.height
      dome.castShadow = true
      group.add(dome)
      const spire = new THREE.Mesh(new THREE.ConeGeometry(0.18, template.roofHeight, 8), materials.solid(template.trimColor))
      spire.position.y = template.height + template.roofHeight / 2
      spire.castShadow = true
      group.add(spire)
      // 旋转光晕环 + 悬停记忆光点：整组绕 Y 自转（spinAxis='y'），光点离散 → 一眼看出在转
      const halo = new THREE.Group()
      halo.position.y = template.height + template.roofHeight * 0.55
      for (let r = 0; r < detail.rings; r++) {
        const ring = new THREE.Mesh(
          new THREE.TorusGeometry(template.width * (0.9 + r * 0.34), 0.035, 5, 28),
          materials.solid(template.trimColor),
        )
        ring.rotation.x = Math.PI / 2
        halo.add(ring)
      }
      const moteMaterial = glowMaterial(materials, template.glowColor)
      glowMaterials.push(moteMaterial)
      for (let i = 0; i < detail.motes; i++) {
        const angle = (i / detail.motes) * Math.PI * 2
        const radius = template.width * (0.9 + (i % 2) * 0.34)
        const mote = new THREE.Mesh(new THREE.SphereGeometry(0.07, 8, 6), moteMaterial)
        mote.position.set(Math.cos(angle) * radius, hashUnit(i * 5 + 11) * 0.2 - 0.1, Math.sin(angle) * radius)
        tag(mote, { glow: true })
        halo.add(mote)
      }
      tag(halo, { spinner: true, spinAxis: 'y' })
      spinners.push(halo)
      group.add(halo)
      break
    }
  }

  // 烟柱（熔炉/作坊：活动时升起）
  if (kind === 'workshop' && flavor === 'forge') {
    const smokeMaterial = new THREE.MeshBasicMaterial({
      color: 0xdadada,
      transparent: true,
      opacity: 0,
      depthWrite: false,
    })
    for (let i = 0; i < 3; i++) {
      const puff = new THREE.Mesh(new THREE.SphereGeometry(0.18 + i * 0.06, 7, 6), smokeMaterial.clone())
      puff.position.set(-template.width * 0.3, template.height + template.roofHeight * 0.95 + i * 0.36, -template.depth * 0.22)
      tag(puff, { smoke: true })
      smoke.push(puff)
      group.add(puff)
    }
  }

  return { group, glowMaterials, spinners, beam, planks, smoke }
}

/* ─────────────────────────── 道路与货物 ─────────────────────────── */

/** 从主堡到建筑的弧线（抬离地面一点点，避免 z-fighting） */
export function roadCurve(from: THREE.Vector3, to: THREE.Vector3): THREE.QuadraticBezierCurve3 {
  const mid = from.clone().add(to).multiplyScalar(0.5)
  const length = from.distanceTo(to)
  const lift = 0.02
  const control = mid.clone()
  // 向岛心同侧的弧：让道路自然弯曲而不是直线穿建筑
  control.x *= 0.55
  control.z *= 0.55
  control.y = Math.max(0, Math.min(0.6, length * 0.05)) + lift
  return new THREE.QuadraticBezierCurve3(
    new THREE.Vector3(from.x, from.y + lift, from.z),
    control,
    new THREE.Vector3(to.x, to.y + lift, to.z),
  )
}

export function buildRoad(curve: THREE.QuadraticBezierCurve3, materials: IslandMaterialSet, color: number, radius = 0.12): THREE.Mesh {
  const geometry = new THREE.TubeGeometry(curve, 16, radius, 5, false)
  const mesh = new THREE.Mesh(geometry, materials.solid(color))
  mesh.receiveShadow = true
  return mesh
}