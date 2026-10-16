/**
 * fridgeBuilders 单测 —— 在 node 环境断言「数据 → three 场景图」的映射正确。
 *
 * 不创建 WebGLRenderer（那需要 GPU/DOM），只断言：
 * - 38 个节点全部建成可拾取 Mesh；
 * - 门组（upperDoor/lowerDoor）挂在右侧铰链枢轴上，本地坐标 = 世界 base - 枢轴；
 * - 抽屉组有独立锚点 + pullDistance；
 * - fixed/compressor 部件锚点在 base 位置、带 explodeOffset；
 * - 每个 Mesh 都带 userData.partId（raycast 能反查）。
 */
import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { buildFridge, disposeFridge, pickableMeshes, FALLBACK_FRIDGE_PALETTE } from './fridgeBuilders'
import { FRIDGE_ROOT } from '../devices/fridge/FridgeModel'
import { collectPartIds } from '../model/PartNode'

describe('fridgeBuilders — 装配树 → three 场景图', () => {
  const assembly = buildFridge(FALLBACK_FRIDGE_PALETTE)

  it('为 FridgeModel 的每个子节点建一个 PartHandle', () => {
    const expected = collectPartIds(FRIDGE_ROOT).filter((id) => id !== 'fridge-root')
    expect(assembly.parts.size).toBe(expected.length)
    for (const id of expected) {
      expect(assembly.parts.has(id), `missing part ${id}`).toBe(true)
    }
  })

  it('所有可拾取 Mesh 都带 userData.partId', () => {
    const meshes = pickableMeshes(assembly)
    expect(meshes.length).toBeGreaterThanOrEqual(35)
    for (const m of meshes) {
      expect(typeof m.userData.partId).toBe('string')
    }
  })

  it('两个门组都有铰链枢轴，且门体是枢轴的子节点', () => {
    expect(assembly.doorPivots.has('upperDoor')).toBe(true)
    expect(assembly.doorPivots.has('lowerDoor')).toBe(true)
    const upper = assembly.doorPivots.get('upperDoor')!
    const lower = assembly.doorPivots.get('lowerDoor')!
    // 枢轴在右侧铰链 x≈2.9
    expect(upper.position.x).toBeCloseTo(2.9, 3)
    expect(lower.position.x).toBeCloseTo(2.9, 3)
    // upper-door / lower-door 网格必须是枢轴的后代
    const upperDoor = assembly.parts.get('upper-door')!
    const lowerDoor = assembly.parts.get('lower-door')!
    expect(upperDoor.mesh.parent).toBe(upper)
    expect(lowerDoor.mesh.parent).toBe(lower)
  })

  it('门体本地 x 偏移为负（门中心相对右铰链在左侧）', () => {
    const upperDoor = assembly.parts.get('upper-door')!
    // 世界 base.x=0，铰链 x=2.9 → 本地 x = -2.9
    expect(upperDoor.mesh.position.x).toBeCloseTo(-2.9, 3)
  })

  it('门组部件（把手/密封条/门搁架/面板）都挂在同一枢轴下', () => {
    const upperPivot = assembly.doorPivots.get('upperDoor')!
    const doorGroupIds = ['upper-handle', 'upper-gasket', 'door-bin-1', 'door-bin-2', 'display-panel']
    for (const id of doorGroupIds) {
      const h = assembly.parts.get(id)!
      expect(h.mesh.parent, `${id} should hang off upper door pivot`).toBe(upperPivot)
    }
  })

  it('抽屉组有独立锚点 + pullDistance', () => {
    const upper = assembly.parts.get('upper-drawer')!
    const lower = assembly.parts.get('lower-drawer')!
    expect(upper.pullDistance).toBeGreaterThan(0)
    expect(lower.pullDistance).toBeGreaterThan(0)
    expect(assembly.drawerAnchors.has('upperDrawer')).toBe(true)
    expect(assembly.drawerAnchors.has('lowerDrawer')).toBe(true)
  })

  it('fixed/compressor 部件锚点在 base 世界坐标，带 explodeOffset', () => {
    const comp = assembly.parts.get('compressor')!
    expect(comp.anchor.position.x).toBeCloseTo(0, 3)
    expect(comp.anchor.position.y).toBeCloseTo(0.8, 3)
    // explodeOffset 沿 -y -z 飞
    expect(comp.explodeOffset.y).toBeLessThan(0)
    const leftWall = assembly.parts.get('left-wall')!
    expect(leftWall.explodeOffset.x).toBeLessThan(0)
  })

  it('门的 openAngle 与 FridgeModel 数据一致（上门 ~1.83，下门 ~1.57）', () => {
    const upper = assembly.parts.get('upper-door')!
    const lower = assembly.parts.get('lower-door')!
    expect(upper.openAngle).toBeCloseTo(1.83, 2)
    expect(lower.openAngle).toBeCloseTo(1.57, 2)
  })

  it('所有几何都是 BufferGeometry（可 dispose）', () => {
    let geoCount = 0
    assembly.root.traverse((obj) => {
      if (obj instanceof THREE.Mesh) {
        expect(obj.geometry).toBeInstanceOf(THREE.BufferGeometry)
        geoCount++
      }
    })
    expect(geoCount).toBeGreaterThanOrEqual(35)
  })

  it('disposeFridge 不抛错（共享材质 + 几何回收路径存在）', () => {
    const local = buildFridge()
    expect(() => disposeFridge(local)).not.toThrow()
  })
})
