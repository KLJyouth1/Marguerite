/**
 * 3D 岛屿地形：种子化噪声高度场 + 水面（纯 three，无 react/r3f）。
 *
 * 为什么程序化几何体而非外部素材？
 *   1) 「AI 自主设计」= 设计是**代码契约**（本文件的高度函数/配色/植被密度全部
 *      可被 Agent 读懂、改写、热更新），而不是几百 MB 美术资产里的隐式知识；
 *   2) 零素材依赖：不需要下载/打包 glTF 就能出世界，构建产物保持轻量；
 *   3) 可热更新：世界 spec 换种子/主题 → 高度场与植被立即重算，无需重新建模。
 *   Blender 管线（blender/generate_island.py）是**升级出口**而非依赖——
 *   需要影视级资产时用它导出 glTF 替换本模块，P0 不依赖 Blender。
 *
 * 地形方案：value-noise（FNV-1a 种子化）+ fbm 多倍频叠加 + 岛屿径向衰减，
 * 高度函数 `heightAt(x, z)` 纯函数供相机贴地/道具落点/植被散布复用。
 */

import * as THREE from 'three'

/** 世界种子（可复现，同 seed → 同地形）。与 worldSpec.defaultWorldSpec().seed 同值。 */
export const DEFAULT_TERRAIN_SEED = 770123

/** 岛屿半径（米）。超出此半径高度快速沉入水下。 */
export const ISLAND_RADIUS = 42

/** 海平面高度（米）。 */
export const WATER_LEVEL = 0

/** 地形网格分辨率（每边顶点数）。64 顶点 ≈ 8K 三角面，远低于 ≤500K 预算。 */
export const TERRAIN_SEGMENTS = 64

/** 噪声高度场最大起伏（米）。 */
const TERRAIN_AMPLITUDE = 7

/** FNV-1a 32 位散列（与 propsSpec.buildPropPlacements 同一算法，保持全库一致）。 */
function fnv1a(input: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < input.length; i += 1) {
    h ^= input.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}

/** 把 [0, 2^32) 散列映射到 [0, 1)。 */
function hashToUnit(h: number): number {
  return (h >>> 0) / 0x100000000
}

/**
 * 2D value-noise：整数格点取种子化散列值，双线性平滑插值。
 * 不用 simplex 库的理由：零依赖、确定性、够用（fbm 叠加后视觉足够有机）。
 */
function valueNoise2D(x: number, z: number, seed: number): number {
  const x0 = Math.floor(x)
  const z0 = Math.floor(z)
  const fx = x - x0
  const fz = z - z0
  // 五次平滑插值（smoothstep 的平方扩展），消除网格棱线。
  const ux = fx * fx * fx * (fx * (fx * 6 - 15) + 10)
  const uz = fz * fz * fz * (fz * (fz * 6 - 15) + 10)
  const v = (ix: number, iz: number) => hashToUnit(fnv1a(`${seed}:${ix}:${iz}`))
  const a = v(x0, z0)
  const b = v(x0 + 1, z0)
  const c = v(x0, z0 + 1)
  const d = v(x0 + 1, z0 + 1)
  return a + (b - a) * ux + (c - a) * uz + (a - b - c + d) * ux * uz
}

/** 分形布朗运动（4 倍频），把 value-noise 叠加成有细节的起伏。 */
function fbm2D(x: number, z: number, seed: number): number {
  let sum = 0
  let amp = 0.5
  let freq = 1
  for (let o = 0; o < 4; o += 1) {
    sum += valueNoise2D(x * freq, z * freq, seed + o * 101) * amp
    amp *= 0.5
    freq *= 2
  }
  return sum // 期望值约 0.94 归一到 0..1 区间近似
}

/**
 * 高度场采样（纯函数）：世界坐标 (x, z) → 地面高度 y（米）。
 * 岛屿形态：径向衰减（中心高、边缘沉入水下）× fbm 起伏 + 中央山峰。
 */
export function heightAt(x: number, z: number, seed: number = DEFAULT_TERRAIN_SEED): number {
  const r = Math.hypot(x, z)
  // 径向衰减：r/ISLAND_RADIUS > 1 时指数沉降，形成海岸线。
  const falloff = Math.max(0, 1 - (r / ISLAND_RADIUS) ** 2)
  const noise = fbm2D(x * 0.045, z * 0.045, seed)
  // 中央山峰（枢轴庭背后的主峰），让天际线有焦点。
  const peak = Math.max(0, 1 - r / (ISLAND_RADIUS * 0.55)) ** 2 * 4
  return falloff * (TERRAIN_AMPLITUDE * noise + peak) - 1.2
}

/** 地形几何颜色（低多边形分层着色：沙滩/草地/岩壁/雪顶）。 */
function terrainColor(y: number, slope: number): THREE.Color {
  // slope = 1 → 平地，0 → 陡壁
  if (y < 0.6) return new THREE.Color(0xd9c48b) // 沙滩
  if (y < 3.2) return new THREE.Color(0x4e7d43) // 草地
  if (y < 5.6) return new THREE.Color(0x6b6f52) // 岩台
  return slope > 0.55 ? new THREE.Color(0x8d9477) : new THREE.Color(0xd8dde2) // 岩壁/雪顶
}

export interface TerrainMeshes {
  terrain: THREE.Mesh
  water: THREE.Mesh
  /** 地面高度采样（相机贴地/道具落点复用同一函数） */
  heightAt: (x: number, z: number) => number
  dispose: () => void
}

/**
 * 生成岛屿地形与水面网格。
 * @param seed 世界种子（可复现）
 * @returns 网格 + 高度采样 + dispose
 */
export function createTerrain(seed: number = DEFAULT_TERRAIN_SEED): TerrainMeshes {
  const segs = TERRAIN_SEGMENTS
  const size = ISLAND_RADIUS * 2.4 // 地形网格略大于可视岛半径，留出海岸缓冲
  const step = size / segs

  const geometry = new THREE.PlaneGeometry(size, size, segs, segs)
  geometry.rotateX(-Math.PI / 2) // XZ 平面朝上

  const pos = geometry.attributes.position as THREE.BufferAttribute
  const colors = new Float32Array(pos.count * 3)
  for (let i = 0; i < pos.count; i += 1) {
    const x = pos.getX(i)
    const z = pos.getZ(i)
    const y = heightAt(x, z, seed)
    pos.setY(i, y)
    // 近似坡度：比较邻域高度差（低多边形风格下的粗略斜率）
    const slope = Math.min(1, Math.max(0, 1 - Math.abs(heightAt(x + step, z, seed) - y) * 1.8))
    const c = terrainColor(y, slope)
    colors[i * 3] = c.r
    colors[i * 3 + 1] = c.g
    colors[i * 3 + 2] = c.b
  }
  geometry.setAttribute('color', new THREE.BufferAttribute(colors, 3))
  geometry.computeVertexNormals()

  const terrainMaterial = new THREE.MeshToonMaterial({ vertexColors: true })
  const terrain = new THREE.Mesh(geometry, terrainMaterial)
  terrain.receiveShadow = true

  // 水面：大平面 + 半透明 toon 材质（低多边形海），轻微顶点起伏做波光。
  const waterGeometry = new THREE.PlaneGeometry(size * 1.6, size * 1.6, 24, 24)
  waterGeometry.rotateX(-Math.PI / 2)
  const wpos = waterGeometry.attributes.position as THREE.BufferAttribute
  for (let i = 0; i < wpos.count; i += 1) {
    wpos.setY(i, Math.sin(wpos.getX(i) * 0.18) * 0.08 + Math.cos(wpos.getZ(i) * 0.22) * 0.08)
  }
  waterGeometry.computeVertexNormals()
  const waterMaterial = new THREE.MeshToonMaterial({
    color: 0x3a7ca5,
    transparent: true,
    opacity: 0.82,
  })
  const water = new THREE.Mesh(waterGeometry, waterMaterial)
  water.position.y = WATER_LEVEL

  const sample = (x: number, z: number) => heightAt(x, z, seed)

  return {
    terrain,
    water,
    heightAt: sample,
    dispose: () => {
      geometry.dispose()
      terrainMaterial.dispose()
      waterGeometry.dispose()
      waterMaterial.dispose()
    },
  }
}
