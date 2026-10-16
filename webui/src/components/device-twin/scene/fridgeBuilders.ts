/**
 * fridgeBuilders —— 从 FridgeModel.ts 的装配树程序化构建 three.js 场景图。
 *
 * 本文件是**纯构造**：只把 PartNode 数据映射成 THREE.Group/Mesh/Geometry，
 * 不创建 WebGLRenderer、不碰 DOM、不读 RAF —— 因此可在 node 单测里断言
 * 「38 个节点都建成了几何、门组挂在铰链枢轴上、爆炸偏移就位」。
 *
 * 与 island/islandBuilders.ts 同一范式：
 * - 几何尺寸/位置/爆炸偏移全部来自数据（FridgeModel），这里不发明尺寸；
 * - 颜色经 colorRole 语义查表（PALETTE），渲染层可 setTheme 覆盖 accent；
 * - 材质按角色共享，选中高亮时按 part clone 一份改 emissive（38 件，开销可忽略）。
 *
 * 坐标约定与 FridgeModel 一致：原点在冰箱底面中心，x=宽（右+），y=高（上+），z=深（前+）。
 * 单位 0.1m。整机 x∈[-3,3], y∈[0,17], z∈[-3,3]。
 */

import * as THREE from 'three'
import { FRIDGE_ROOT } from '../devices/fridge/FridgeModel'
import type { KinematicGroup, PartColorRole, PartNode } from '../model/PartNode'

/** colorRole → three 颜色（中性工业灰阶；accent 由渲染层从 CSS 变量注入） */
export interface FridgePalette {
  body: number
  interior: number
  metal: number
  rubber: number
  glass: number
  copper: number
  hot: number
  cold: number
  accent: number
}

/** 离线兜底调色板（与 dark theme 同色系；CSS 变量读失败时使用） */
export const FALLBACK_FRIDGE_PALETTE: FridgePalette = {
  body: 0xcfd4d9,
  interior: 0xeef1f4,
  metal: 0x9aa0a6,
  rubber: 0x3a3d42,
  glass: 0xbcdcec,
  copper: 0xb87333,
  hot: 0xe8734a,
  cold: 0x56c2e6,
  accent: 0x60a5fa,
}

/**
 * 单个部件的句柄。渲染层每帧据此插值：
 * - pivot.position  = base + explodeOffset * explodeProgress (+ drawer pull)
 * - pivot.rotation.y = openAngle * doorOpen（仅门组）
 */
export interface PartHandle {
  id: string
  group: KinematicGroup
  node: PartNode
  /** 锚点（门组 = 铰链枢轴；其余 = 部件中心）。世界坐标。 */
  anchor: THREE.Group
  /** 可视网格（raycast 目标；userData.partId 已写入） */
  mesh: THREE.Object3D
  /** 锚点基准位置（未爆炸） */
  base: THREE.Vector3
  /** 爆炸偏移 */
  explodeOffset: THREE.Vector3
  /** 门组：绕铰链枢轴的最大开合角（弧度）；其余为 0 */
  openAngle: number
  /** 抽屉组：沿 +z 拉出距离；其余为 0 */
  pullDistance: number
  /** 共享材质（高亮时 clone） */
  material: THREE.MeshStandardMaterial
}

export interface FridgeAssembly {
  root: THREE.Group
  parts: Map<string, PartHandle>
  /** 门组铰链枢轴（按 group 索引，便于渲染层批量旋转） */
  doorPivots: Map<'upperDoor' | 'lowerDoor', THREE.Group>
  /** 抽屉组锚点（按 group 索引） */
  drawerAnchors: Map<'upperDrawer' | 'lowerDrawer', THREE.Group>
  /** 共享材质集（dispose 时统一回收） */
  materials: THREE.MeshStandardMaterial[]
}

/** 铰链在右侧（x≈+2.9）。门组所有子件都挂到这个 x 位置的枢轴上。 */
const HINGE_X = 2.9

function toVec3(tuple: readonly [number, number, number]): THREE.Vector3 {
  return new THREE.Vector3(tuple[0], tuple[1], tuple[2])
}

/** 按 colorRole 取材质；同角色共享一个实例。 */
function materialFor(role: PartColorRole, palette: FridgePalette, cache: Map<PartColorRole, THREE.MeshStandardMaterial>, out: THREE.MeshStandardMaterial[]): THREE.MeshStandardMaterial {
  const hit = cache.get(role)
  if (hit) return hit
  const color = palette[role]
  const isGlass = role === 'glass'
  const mat = new THREE.MeshStandardMaterial({
    color,
    roughness: role === 'metal' || role === 'copper' ? 0.45 : 0.75,
    metalness: role === 'metal' || role === 'copper' ? 0.6 : 0.05,
    transparent: isGlass,
    opacity: isGlass ? 0.35 : 1,
    depthWrite: !isGlass,
  })
  cache.set(role, mat)
  out.push(mat)
  return mat
}

/** 按 geometry.kind 生成几何（全部程序化，无外部 GLB）。 */
function buildGeometry(node: PartNode): THREE.BufferGeometry {
  const g = node.geometry
  switch (g.kind) {
    case 'box':
    case 'panel':
    case 'shelf': {
      const [w, h, d] = g.size as [number, number, number]
      return new THREE.BoxGeometry(w, h, d)
    }
    case 'cylinder': {
      const [radius, height] = g.size as [number, number]
      return new THREE.CylinderGeometry(radius, radius, height, 18)
    }
    case 'compressor': {
      // 压缩机 = 圆柱壳体 + 顶部圆顶（用圆柱近似，避免引入球体几何开销）
      const [w, h, d] = g.size as [number, number, number]
      return new THREE.CylinderGeometry(Math.min(w, d) * 0.45, Math.min(w, d) * 0.5, h, 20)
    }
    case 'tube': {
      // 弯管：用一小段 4 点 CatmullRom 曲线 + TubeGeometry 表示「这里有根盘管」。
      // 具体管路在工作循环粒子层（FRIDGE_FLOW_CHANNELS）里走，这里只占位成可视件。
      const curve = new THREE.CatmullRomCurve3([
        new THREE.Vector3(-1.2, -0.6, 0),
        new THREE.Vector3(0, -0.3, 0.2),
        new THREE.Vector3(0, 0.3, -0.2),
        new THREE.Vector3(1.2, 0.6, 0),
      ])
      return new THREE.TubeGeometry(curve, 20, 0.08, 8, false)
    }
  }
}

/**
 * 构建整台冰箱。
 *
 * 分组规则（读 PartNode.group）：
 * - upperDoor / lowerDoor：挂到右侧铰链枢轴（绕 Y 旋转开门）；
 *   同组所有部件（门体/把手/密封条/门搁架/面板）都成为枢轴的子节点，
 *   本地坐标 = 其世界 base - 铰链枢轴位置。
 * - upperDrawer / lowerDrawer：抽屉锚点，沿 +z 拉出。
 * - compressor / fixed：直接挂 root，按 explodeOffset 径向偏移。
 */
export function buildFridge(palette: FridgePalette = FALLBACK_FRIDGE_PALETTE): FridgeAssembly {
  const root = new THREE.Group()
  root.name = 'fridge-root'
  const parts = new Map<string, PartHandle>()
  const doorPivots = new Map<'upperDoor' | 'lowerDoor', THREE.Group>()
  const drawerAnchors = new Map<'upperDrawer' | 'lowerDrawer', THREE.Group>()
  const materials: THREE.MeshStandardMaterial[] = []
  const materialCache = new Map<PartColorRole, THREE.MeshStandardMaterial>()

  // 为两个门预建铰链枢轴。枢轴 y 取门体中心高度（与 metadata base.y 一致）。
  const upperPivot = new THREE.Group()
  upperPivot.position.set(HINGE_X, 11.6, 3.05)
  upperPivot.name = 'pivot-upperDoor'
  const lowerPivot = new THREE.Group()
  lowerPivot.position.set(HINGE_X, 4.1, 3.05)
  lowerPivot.name = 'pivot-lowerDoor'
  root.add(upperPivot, lowerPivot)
  doorPivots.set('upperDoor', upperPivot)
  doorPivots.set('lowerDoor', lowerPivot)

  for (const child of FRIDGE_ROOT.children ?? []) {
    const mat = materialFor(child.geometry.colorRole, palette, materialCache, materials)
    const geo = buildGeometry(child)
    const mesh = new THREE.Mesh(geo, mat)
    mesh.name = child.id
    mesh.userData.partId = child.id

    const base = toVec3(child.transform.base)
    const explode = toVec3(child.transform.explodeOffset)

    if (child.group === 'upperDoor' || child.group === 'lowerDoor') {
      const pivot = child.group === 'upperDoor' ? upperPivot : lowerPivot
      // 本地坐标 = 世界 base - 枢轴位置（枢轴在铰链处）
      mesh.position.copy(base).sub(pivot.position)
      pivot.add(mesh)
      parts.set(child.id, {
        id: child.id,
        group: child.group,
        node: child,
        anchor: pivot,
        mesh,
        base: pivot.position.clone(),
        explodeOffset: explode,
        openAngle: child.transform.openAxis === 'y' ? child.transform.openAngle ?? 0 : 0,
        pullDistance: 0,
        material: mat,
      })
    } else if (child.group === 'upperDrawer' || child.group === 'lowerDrawer') {
      const anchor = new THREE.Group()
      anchor.position.copy(base)
      anchor.name = `anchor-${child.group}`
      mesh.position.set(0, 0, 0)
      anchor.add(mesh)
      root.add(anchor)
      drawerAnchors.set(child.group, anchor)
      parts.set(child.id, {
        id: child.id,
        group: child.group,
        node: child,
        anchor,
        mesh,
        base: base.clone(),
        explodeOffset: explode,
        openAngle: 0,
        pullDistance: child.transform.pullAxis === 'z' ? child.transform.pullDistance ?? 0 : 0,
        material: mat,
      })
    } else {
      // fixed / compressor
      const anchor = new THREE.Group()
      anchor.position.copy(base)
      anchor.name = `anchor-${child.id}`
      mesh.position.set(0, 0, 0)
      anchor.add(mesh)
      root.add(anchor)
      parts.set(child.id, {
        id: child.id,
        group: child.group,
        node: child,
        anchor,
        mesh,
        base: base.clone(),
        explodeOffset: explode,
        openAngle: 0,
        pullDistance: 0,
        material: mat,
      })
    }
  }

  return { root, parts, doorPivots, drawerAnchors, materials }
}

/** 递归收集场景里所有带 userData.partId 的可拾取对象（raycast 用）。 */
export function pickableMeshes(assembly: FridgeAssembly): THREE.Object3D[] {
  const out: THREE.Object3D[] = []
  assembly.root.traverse((obj) => {
    if ((obj as THREE.Mesh).isMesh && typeof obj.userData.partId === 'string') out.push(obj)
  })
  return out
}

/**
 * 销毁装配：dispose 几何 + 共享材质。渲染层 dispose() 时调用。
 */
export function disposeFridge(assembly: FridgeAssembly): void {
  assembly.root.traverse((obj) => {
    if (obj instanceof THREE.Mesh) obj.geometry.dispose()
  })
  for (const mat of assembly.materials) mat.dispose()
}
