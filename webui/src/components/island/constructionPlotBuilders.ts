/**
 * 施工地块几何构建器 —— 把工序快照变成 Three.js 网格。
 *
 * 铁律落地：
 * - **绝不 scaleY 拉伸建筑本体**：柱/梁/板按楼层整层预建，按 `visibleFloors`（整数）
 *   整层增删。一层楼板是一整块真几何，不是把成品盒子沿 Y 拉长。
 * - **重复件 InstancedMesh**：脚手架黄管、锥桶围挡各一条 InstancedMesh 吃全部实例；
 *   工人数 ≤ crew，也走一条 InstancedMesh。
 * - **配色是 3D 数据层整数**（CONSTRUCTION_PALETTE / spec.palette），不进 CSS。
 * - **动画数学全在 constructionSystem 纯函数里**：本文件只把姿态数写进网格。
 */

import * as THREE from 'three'
import type { ConstructionPlotConfig, StageSnapshot } from './buildingGrowth'
import {
  barrierRing,
  CONSTRUCTION_PALETTE,
  cranePoseAt,
  scaffoldGrid,
  vehiclePoseAt,
  workerPoseAt,
  type WorkerWaypoint,
} from './constructionSystem'
import type { IslandMaterialSet } from './islandBuilders'
import { hashUnit } from './islandSim'

const dummy = new THREE.Object3D()

/** 施工地块句柄：渲染层按快照逐部件显隐，按真实时间驱动塔吊/工人/车辆 */
export interface ConstructionPlotHandle {
  group: THREE.Group
  config: ConstructionPlotConfig
  foundation: THREE.Mesh
  /** 每层一套（柱/梁/板），下标 = 楼层；visibleFloors 决定前几层可见 */
  floors: { column: THREE.Group; beam: THREE.Group; slab: THREE.Mesh }[]
  walls: THREE.Group
  windows: THREE.Group
  interior: THREE.Group
  dressing: THREE.Group
  scaffold: THREE.InstancedMesh
  barriers: THREE.InstancedMesh
  cranePivot: THREE.Group
  craneTrolley: THREE.Mesh
  craneCable: THREE.Mesh
  craneHook: THREE.Mesh
  craneHookLoad: THREE.Mesh
  craneJibLength: number
  craneHeight: number
  workers: THREE.InstancedMesh
  vehicles: THREE.Group[]
  workerWaypoints: WorkerWaypoint[]
  vehicleLoop: { x: number; z: number }[]
  workerOffsets: number[]
  vehicleOffsets: number[]
  craneOffset: number
  /** 上一次重建脚手架时的楼层数（整数阶跃时才重建，不每帧重建矩阵） */
  lastScaffoldFloors: number
}

/** 构建一个施工地块的全部部件（满配预建，后续只切 visible，不重建几何） */
export function buildConstructionPlot(
  config: ConstructionPlotConfig,
  materials: IslandMaterialSet,
): ConstructionPlotHandle {
  const group = new THREE.Group()
  const f = config.building.footprint
  const h = config.building.floorHeight
  const N = Math.max(1, config.building.totalFloors)

  const concrete = materials.solid(CONSTRUCTION_PALETTE.concrete)
  const steel = materials.solid(0x9a9a92)
  const formwork = materials.solid(CONSTRUCTION_PALETTE.formwork)

  // ── 承台地基 ──
  const foundation = new THREE.Mesh(new THREE.BoxGeometry(f + 0.5, 0.3, f + 0.5), concrete)
  foundation.position.y = 0.15
  foundation.receiveShadow = true
  group.add(foundation)

  // ── 逐层柱/梁/板（整层预建，按 visibleFloors 显隐，绝不拉伸）──
  const floors: ConstructionPlotHandle['floors'] = []
  const corner = f / 2 - 0.09
  for (let i = 0; i < N; i++) {
    const column = new THREE.Group()
    // 四角柱
    for (const sx of [-1, 1]) {
      for (const sz of [-1, 1]) {
        const col = new THREE.Mesh(new THREE.BoxGeometry(0.16, h, 0.16), steel)
        col.position.set(sx * corner, i * h + h / 2, sz * corner)
        col.castShadow = true
        column.add(col)
      }
    }
    // 梁（本层柱顶一圈）
    const beam = new THREE.Group()
    const yBeam = (i + 1) * h - 0.07
    const fb = new THREE.Mesh(new THREE.BoxGeometry(f, 0.14, 0.16), steel)
    fb.position.set(0, yBeam, corner)
    beam.add(fb)
    const bb = fb.clone()
    bb.position.z = -corner
    beam.add(bb)
    const lr = new THREE.Mesh(new THREE.BoxGeometry(0.16, 0.14, f), steel)
    lr.position.set(corner, yBeam, 0)
    beam.add(lr)
    const rl = lr.clone()
    rl.position.x = -corner
    beam.add(rl)
    // 楼板（本层顶板）
    const slab = new THREE.Mesh(new THREE.BoxGeometry(f, 0.12, f), concrete)
    slab.position.y = (i + 1) * h
    slab.receiveShadow = true
    group.add(column, beam, slab)
    floors.push({ column, beam, slab })
  }

  // ── 外墙（facade 阶段封闭；按层分板，整组显隐）──
  const walls = new THREE.Group()
  const wallMat = materials.solid(0xd8d2c4)
  for (let i = 0; i < N; i++) {
    const y = (i + 0.5) * h
    const front = new THREE.Mesh(new THREE.BoxGeometry(f * 0.92, h * 0.9, 0.08), wallMat)
    front.position.set(0, y, f / 2 - 0.04)
    walls.add(front)
    const back = front.clone()
    back.position.z = -(f / 2 - 0.04)
    walls.add(back)
    const left = new THREE.Mesh(new THREE.BoxGeometry(0.08, h * 0.9, f * 0.92), wallMat)
    left.position.set(f / 2 - 0.04, y, 0)
    walls.add(left)
    const right = left.clone()
    right.position.x = -(f / 2 - 0.04)
    walls.add(right)
  }
  walls.visible = false
  group.add(walls)

  // ── 窗格（windows 阶段安装；浅色玻璃块）──
  const windows = new THREE.Group()
  const glassMat = materials.solid(0xaec6d8)
  for (let i = 0; i < N; i++) {
    const y = (i + 0.55) * h
    for (const side of [-1, 1]) {
      const pane = new THREE.Mesh(new THREE.BoxGeometry(0.26, 0.3, 0.04), glassMat)
      pane.position.set(0, y, side * (f / 2 - 0.09))
      windows.add(pane)
    }
  }
  windows.visible = false
  group.add(windows)

  // ── 内部/收尾（interior：平屋顶 + 门）──
  const interior = new THREE.Group()
  const roof = new THREE.Mesh(new THREE.BoxGeometry(f * 1.04, 0.16, f * 1.04), formwork)
  roof.position.y = N * h + 0.08
  roof.castShadow = true
  interior.add(roof)
  const door = new THREE.Mesh(new THREE.BoxGeometry(0.34, 0.62, 0.06), materials.solid(0x6f4a2a))
  door.position.set(0, 0.31, f / 2 + 0.01)
  interior.add(door)
  interior.visible = false
  group.add(interior)

  // ── 完工 dressing：围挡拆、加小灌木 ──
  const dressing = new THREE.Group()
  const shrubMat = materials.solid(0x5aa851)
  for (let i = 0; i < 4; i++) {
    const angle = (i / 4) * Math.PI * 2 + 0.4
    const shrub = new THREE.Mesh(new THREE.ConeGeometry(0.16, 0.34, 6), shrubMat)
    shrub.position.set(Math.cos(angle) * (f / 2 + 0.5), 0.17, Math.sin(angle) * (f / 2 + 0.5))
    dressing.add(shrub)
  }
  dressing.visible = false
  group.add(dressing)

  // ── 脚手架（InstancedMesh 黄管；随 visibleFloors 重建高度）──
  const scaffoldGeo = new THREE.CylinderGeometry(0.035, 0.035, 1, 5)
  const scaffold = new THREE.InstancedMesh(scaffoldGeo, materials.solid(CONSTRUCTION_PALETTE.steelYellow), 4 * 4)
  scaffold.visible = false
  group.add(scaffold)

  // ── 锥桶围挡（InstancedMesh 安全橙；完工后拆）──
  const barrierGeo = new THREE.ConeGeometry(0.09, 0.28, 6)
  const barrierData = barrierRing(f)
  const barriers = new THREE.InstancedMesh(barrierGeo, materials.solid(CONSTRUCTION_PALETTE.safetyOrange), barrierData.length)
  barrierData.forEach((t, i) => {
    dummy.position.set(t.x, 0.14, t.z)
    dummy.scale.setScalar(1)
    dummy.rotation.set(0, 0, 0)
    dummy.updateMatrix()
    barriers.setMatrixAt(i, dummy.matrix)
  })
  barriers.instanceMatrix.needsUpdate = true
  group.add(barriers)

  // ── 塔吊（侧置；臂架绕 Y、小车外伸、吊钩升降）──
  const cranePivot = new THREE.Group()
  const craneHeight = N * h + 0.7
  const mast = new THREE.Mesh(new THREE.BoxGeometry(0.18, craneHeight, 0.18), materials.solid(CONSTRUCTION_PALETTE.safetyOrange))
  mast.position.y = craneHeight / 2
  cranePivot.add(mast)
  cranePivot.position.set(f / 2 + 0.7, 0, f / 2 + 0.4)
  const jibLength = f * 1.1
  const jib = new THREE.Mesh(new THREE.BoxGeometry(jibLength, 0.07, 0.07), materials.solid(CONSTRUCTION_PALETTE.safetyOrange))
  jib.position.x = jibLength / 2
  cranePivot.add(jib)
  const counter = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.12, 0.12), materials.solid(0x7d7d7d))
  counter.position.x = -0.25
  cranePivot.add(counter)
  const craneTop = new THREE.Group()
  craneTop.position.y = craneHeight
  cranePivot.add(craneTop)
  const craneTrolley = new THREE.Mesh(new THREE.BoxGeometry(0.12, 0.1, 0.14), materials.solid(0x333333))
  craneTop.add(craneTrolley)
  const craneCable = new THREE.Mesh(new THREE.CylinderGeometry(0.012, 0.012, 1, 4), materials.solid(0x222222))
  craneTop.add(craneCable)
  const craneHook = new THREE.Mesh(new THREE.BoxGeometry(0.1, 0.12, 0.1), materials.solid(0x222222))
  craneTop.add(craneHook)
  const craneHookLoad = new THREE.Mesh(new THREE.BoxGeometry(0.22, 0.18, 0.22), materials.solid(CONSTRUCTION_PALETTE.formwork))
  craneTop.add(craneHookLoad)
  cranePivot.visible = false
  group.add(cranePivot)

  // ── 工人（InstancedMesh 黄背心小人，沿工地环线走）──
  const workerCount = Math.max(0, Math.round(config.crew))
  const workerGeo = new THREE.CylinderGeometry(0.07, 0.08, 0.3, 5)
  const workers = new THREE.InstancedMesh(workerGeo, materials.solid(CONSTRUCTION_PALETTE.safetyVest), Math.max(1, workerCount))
  workers.count = 0
  group.add(workers)

  // ── 工程车辆（简单卡车盒，沿环线；最多 2 辆）──
  const vehicles: THREE.Group[] = []
  for (let i = 0; i < Math.max(0, Math.round(config.vehicles)); i++) {
    const truck = new THREE.Group()
    const body = new THREE.Mesh(new THREE.BoxGeometry(0.3, 0.2, 0.5), materials.solid(CONSTRUCTION_PALETTE.excavator))
    body.position.y = 0.16
    truck.add(body)
    const cab = new THREE.Mesh(new THREE.BoxGeometry(0.26, 0.16, 0.16), materials.solid(0xdddddd))
    cab.position.set(0, 0.2, 0.26)
    truck.add(cab)
    truck.visible = false
    group.add(truck)
    vehicles.push(truck)
  }

  // 工人环线：绕工地一圈，在四角驻留作业
  const r = f / 2 + 0.6
  const workerWaypoints: WorkerWaypoint[] = [
    { x: -r, z: -r, dwell: 1.5 },
    { x: r, z: -r, dwell: 1.5 },
    { x: r, z: r, dwell: 1.5 },
    { x: -r, z: r, dwell: 1.5 },
  ]
  const vehicleLoop = [
    { x: -r - 0.6, z: -r - 0.6 },
    { x: r + 0.6, z: -r - 0.6 },
    { x: r + 0.6, z: r + 0.6 },
    { x: -r - 0.6, z: r + 0.6 },
  ]
  const seed = hashUnit(config.id.length * 7 + config.x * 13)
  const workerOffsets = Array.from({ length: Math.max(1, workerCount) }, (_, i) => seed * 10 + i * 1.7)
  const vehicleOffsets = vehicles.map((_, i) => seed * 5 + i * 3.3)

  return {
    group,
    config,
    foundation,
    floors,
    walls,
    windows,
    interior,
    dressing,
    scaffold,
    barriers,
    cranePivot,
    craneTrolley,
    craneCable,
    craneHook,
    craneHookLoad,
    craneJibLength: jibLength,
    craneHeight,
    workers,
    vehicles,
    workerWaypoints,
    vehicleLoop,
    workerOffsets,
    vehicleOffsets,
    craneOffset: seed * 20,
    lastScaffoldFloors: -1,
  }
}

/** 按整数楼层重建脚手架管阵列（高度随层数变化；只在 visibleFloors 变化时调） */
function rebuildScaffold(handle: ConstructionPlotHandle): void {
  const f = handle.config.building.footprint
  const h = handle.config.building.floorHeight
  const floors = Math.max(1, handle.lastScaffoldFloors)
  const posts = scaffoldGrid(f, floors, h)
  handle.scaffold.count = posts.length
  posts.forEach((t, i) => {
    dummy.position.set(t.x, t.scale / 2, t.z)
    dummy.scale.set(1, t.scale, 1)
    dummy.rotation.set(0, 0, 0)
    dummy.updateMatrix()
    handle.scaffold.setMatrixAt(i, dummy.matrix)
  })
  handle.scaffold.instanceMatrix.needsUpdate = true
}

/**
 * 把工序快照落到网格：整层增删部件 + 重复件计数。
 * 绝不拉伸建筑本体 —— visibleFloors 是整数，每层柱/梁/板整块显隐。
 */
export function applyConstructionSnapshot(handle: ConstructionPlotHandle, snapshot: StageSnapshot): void {
  const p = snapshot.parts
  handle.foundation.visible = p.foundation
  for (let i = 0; i < handle.floors.length; i++) {
    const on = i < snapshot.visibleFloors
    handle.floors[i].column.visible = on
    handle.floors[i].beam.visible = on
    handle.floors[i].slab.visible = on
  }
  handle.walls.visible = p.walls
  handle.windows.visible = p.windows
  handle.interior.visible = p.interior
  handle.dressing.visible = p.finishedDressing
  handle.barriers.visible = !p.finishedDressing

  handle.scaffold.visible = p.scaffolding
  if (p.scaffolding && snapshot.visibleFloors !== handle.lastScaffoldFloors) {
    handle.lastScaffoldFloors = snapshot.visibleFloors
    rebuildScaffold(handle)
  }

  handle.cranePivot.visible = snapshot.activity.cranes > 0
  const wantWorkers = Math.min(handle.workerOffsets.length, snapshot.activity.workers)
  handle.workers.count = Math.max(0, wantWorkers)
  handle.vehicles.forEach((v, i) => {
    v.visible = i < snapshot.activity.vehicles
  })
}

/** 每帧驱动塔吊/工人/车辆的真实工作循环（动画数学在 constructionSystem 纯函数里） */
export function updateConstructionAnimation(handle: ConstructionPlotHandle, t: number): void {
  if (handle.cranePivot.visible) {
    const pose = cranePoseAt(t, handle.craneOffset)
    handle.cranePivot.rotation.y = pose.armAngle
    const trolleyX = 0.2 + pose.trolley * handle.craneJibLength
    handle.craneTrolley.position.x = trolleyX
    // 吊钩高度：hookY 0=地面 1=塔顶
    const hookTop = handle.craneHeight
    const hookY = hookTop * (0.15 + 0.85 * pose.hookY)
    handle.craneHook.position.set(trolleyX, hookY, 0)
    const cableLen = Math.max(0.05, hookTop - hookY)
    handle.craneCable.position.set(trolleyX, hookY + cableLen / 2, 0)
    handle.craneCable.scale.set(1, cableLen, 1)
    handle.craneHookLoad.visible = pose.loadAttached
    handle.craneHookLoad.position.set(trolleyX, hookY - 0.16, 0)
  }

  if (handle.workers.count > 0) {
    for (let i = 0; i < handle.workers.count; i++) {
      const pose = workerPoseAt(t, handle.workerOffsets[i], handle.workerWaypoints)
      dummy.position.set(pose.x, 0.15, pose.z)
      dummy.rotation.set(0, pose.facing, 0)
      dummy.scale.setScalar(1)
      dummy.updateMatrix()
      handle.workers.setMatrixAt(i, dummy.matrix)
    }
    handle.workers.instanceMatrix.needsUpdate = true
  }

  handle.vehicles.forEach((truck, i) => {
    if (!truck.visible) return
    const pose = vehiclePoseAt(t, handle.vehicleOffsets[i], handle.vehicleLoop)
    truck.position.set(pose.x, 0, pose.z)
    truck.rotation.y = pose.heading
  })
}
