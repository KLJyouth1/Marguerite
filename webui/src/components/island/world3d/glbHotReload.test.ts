/**
 * GLB 热加载消费端测试。
 *
 * ── 真实字节 vs mock ──────────────────────────────────────────────────
 * 这里有一条**真实字节链路**用例：用 node:fs 读 assets/fixtures/*.glb
 * （由 scripts/dev/make_fixture_glb.py 用纯标准库手写出来的合法 GLB，
 * 本机无 Blender），交给真实的 GLTFLoader.parse 解析，再过 validateGlbScene。
 * 这样才能证明「真实 .glb 二进制 → 真实 GLTFLoader → 校验 → 可入库模板」
 * 整条链真的能跑，而不是「我们自己写的 mock 能跑」。
 *
 * 其余用例用注入的 fake loader 覆盖管理器的原子替换 / 失败回退语义，
 * 断言全部基于真实数值（三角形数、位姿、revision、事件次数）。
 *
 * vitest 环境是 node（vite.config.ts test.environment），无 jsdom——不碰 DOM。
 */

import { describe, it, expect, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import {
  DEFAULT_TRIANGLE_BUDGET,
  GlbHotReloadManager,
  validateGlbScene,
  type GlbChangeEvent,
} from './glbHotReload'

const HERE = dirname(fileURLToPath(import.meta.url))
const FIXTURES = resolve(HERE, 'assets', 'fixtures')

/** 读夹具 GLB 的原始字节（ArrayBuffer 视图，供 GLTFLoader.parse 使用）。 */
function fixtureBuffer(name: string): ArrayBuffer {
  const buf = readFileSync(resolve(FIXTURES, name))
  return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer
}

/** 用真实 GLTFLoader 解析一份 GLB 字节，返回 gltf.scene。失败则 reject（不吞错）。 */
function parseGlb(buffer: ArrayBuffer): Promise<THREE.Group> {
  const loader = new GLTFLoader()
  return new Promise<THREE.Group>((resolvePromise, rejectPromise) => {
    loader.parse(buffer, '', (gltf) => resolvePromise(gltf.scene), (err) => rejectPromise(err))
  })
}

function countMeshes(scene: THREE.Object3D): number {
  let n = 0
  scene.traverse((o) => {
    if ((o as THREE.Mesh).isMesh) n += 1
  })
  return n
}

function countTriangles(scene: THREE.Object3D): number {
  let tris = 0
  scene.traverse((o) => {
    const mesh = o as THREE.Mesh
    if (!mesh.isMesh) return
    const index = mesh.geometry.index
    tris += index ? Math.floor(index.count / 3) : 0
  })
  return tris
}

/** 非退化合法网格（立方体，12 三角形，1×1×1 包围盒）。 */
function boxMesh(): THREE.Mesh {
  return new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial())
}

function sceneWith(mesh: THREE.Mesh): THREE.Group {
  const group = new THREE.Group()
  group.add(mesh)
  return group
}

type LoaderArg = Pick<GLTFLoader, 'loadAsync'>

/** 注入用 fake loader：resolveScene 返回值即 gltf.scene；抛错即模拟加载失败。 */
function fakeLoader(resolveScene: (url: string) => THREE.Object3D, onCall?: (url: string) => void): LoaderArg {
  return {
    loadAsync: async (url: string) => {
      onCall?.(url)
      return { scene: resolveScene(url) }
    },
  } as unknown as LoaderArg
}

function failingLoader(message = 'HTTP 404'): LoaderArg {
  return {
    loadAsync: async () => {
      throw new Error(message)
    },
  } as unknown as LoaderArg
}

// ───────────────────────── 真实字节链路 ─────────────────────────

describe('真实字节链路：GLB 文件 → GLTFLoader.parse → validateGlbScene', () => {
  it('code_crystal.glb 经真实 GLTFLoader 解析后通过校验并给出模板', async () => {
    const scene = await parseGlb(fixtureBuffer('code_crystal.glb'))
    const res = validateGlbScene(scene, 'code_crystal')

    expect(res.ok).toBe(true)
    expect(res.template).not.toBeNull()
    expect(res.template).toBe(scene)
    expect(res.errors).toEqual([])
  })

  it('code_crystal.glb 解析出真实 Mesh 数与三角形数（1 个 Mesh / 32 三角形）', async () => {
    const scene = await parseGlb(fixtureBuffer('code_crystal.glb'))

    expect(countMeshes(scene)).toBe(1)
    expect(countTriangles(scene)).toBe(32)
  })

  it('code_crystal.glb 的 POSITION/NORMAL/索引为真实字节落地（18 顶点 / UNSIGNED_SHORT / ±0.8 包围盒）', async () => {
    const scene = await parseGlb(fixtureBuffer('code_crystal.glb'))

    let mesh: THREE.Mesh | null = null
    scene.traverse((o) => {
      if ((o as THREE.Mesh).isMesh) mesh = o as THREE.Mesh
    })
    expect(mesh).not.toBeNull()

    const geometry = (mesh as unknown as THREE.Mesh).geometry
    const position = geometry.getAttribute('position')
    const normal = geometry.getAttribute('normal')
    expect(position).toBeDefined()
    expect(normal).toBeDefined()
    expect(position.count).toBe(18)

    expect(geometry.index).not.toBeNull()
    expect(geometry.index?.array).toBeInstanceOf(Uint16Array)
    expect(geometry.index?.count).toBe(96)

    geometry.computeBoundingBox()
    const box = geometry.boundingBox
    expect(box).not.toBeNull()
    // 八面体 scale=0.8 → 三轴都精确落在 -0.8..0.8
    expect(box?.min.x).toBeCloseTo(-0.8, 5)
    expect(box?.max.x).toBeCloseTo(0.8, 5)
    expect(box?.min.y).toBeCloseTo(-0.8, 5)
    expect(box?.max.y).toBeCloseTo(0.8, 5)
    expect(box?.min.z).toBeCloseTo(-0.8, 5)
    expect(box?.max.z).toBeCloseTo(0.8, 5)
  })

  it('oversized.glb 真实面数 60800 > 默认预算，判定 triangle_budget_exceeded', async () => {
    const scene = await parseGlb(fixtureBuffer('oversized.glb'))
    const triangles = countTriangles(scene)

    // 真实数值：脚本 --budget-bust 生成的 320×96 段球体
    expect(triangles).toBe(60800)
    expect(triangles).toBeGreaterThan(DEFAULT_TRIANGLE_BUDGET)

    const res = validateGlbScene(scene, 'code_crystal')
    expect(res.ok).toBe(false)
    expect(res.template).toBeNull()
    expect(res.errors).toHaveLength(1)
    expect(res.errors[0].code).toBe('triangle_budget_exceeded')
    expect(res.errors[0].path).toBe('code_crystal')
    // 错误信息里带着真实面数与默认预算
    expect(res.errors[0].message).toContain('60800')
    expect(res.errors[0].message).toContain(String(DEFAULT_TRIANGLE_BUDGET))
  })

  it('oversized.glb 注入极小预算 {triangleBudget:100} 同样判超预算（明确是注入预算，非默认值）', async () => {
    const scene = await parseGlb(fixtureBuffer('oversized.glb'))
    const res = validateGlbScene(scene, 'code_crystal', { triangleBudget: 100 })

    expect(res.ok).toBe(false)
    expect(res.errors[0].code).toBe('triangle_budget_exceeded')
    expect(res.errors[0].message).toContain('100')
    expect(res.errors[0].message).not.toContain(String(DEFAULT_TRIANGLE_BUDGET))
  })
})

// ───────────────────────── 校验器单元用例 ─────────────────────────

describe('validateGlbScene 单元用例', () => {
  it('空场景（无 Mesh）→ no_mesh', () => {
    const res = validateGlbScene(new THREE.Group(), 'code_crystal')

    expect(res.ok).toBe(false)
    expect(res.template).toBeNull()
    expect(res.errors).toHaveLength(1)
    expect(res.errors[0].code).toBe('no_mesh')
    expect(res.errors[0].path).toBe('code_crystal')
  })

  it('退化包围盒（平面三角 z=0，至少一轴为 0）→ degenerate_bounds', () => {
    const geometry = new THREE.BufferGeometry()
    geometry.setAttribute('position', new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 0, 1, 0], 3))
    geometry.setIndex([0, 1, 2])

    const res = validateGlbScene(new THREE.Mesh(geometry), 'code_crystal')

    expect(res.ok).toBe(false)
    expect(res.template).toBeNull()
    expect(res.errors).toHaveLength(1)
    expect(res.errors[0].code).toBe('degenerate_bounds')
  })

  it('面数超出注入预算（12 三角形 vs 注入 10）→ triangle_budget_exceeded，且报的是注入值', () => {
    const res = validateGlbScene(boxMesh(), 'code_crystal', { triangleBudget: 10 })

    expect(res.ok).toBe(false)
    expect(res.template).toBeNull()
    expect(res.errors[0].code).toBe('triangle_budget_exceeded')
    expect(res.errors[0].message).toContain('12')
    expect(res.errors[0].message).toContain('10')
  })

  it('合法非退化网格通过校验（ok / 模板非空 / 无错误无警告）', () => {
    const res = validateGlbScene(sceneWith(boxMesh()), 'code_crystal')

    expect(res.ok).toBe(true)
    expect(res.template).not.toBeNull()
    expect(res.errors).toEqual([])
    expect(res.warnings).toEqual([])
  })
})

// ───────────────────────── 管理器用例（注入 fake loader）─────────────────────────

describe('GlbHotReloadManager.loadPropAssets（注入 fake loader）', () => {
  it('成功加载：revision 递增，subscribe 收到一次通知（kinds 为真实加载项）', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './a.glb')
    const events: GlbChangeEvent[] = []
    mgr.subscribe((e) => events.push(e))

    const first = await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh())))
    expect(first.ok).toBe(true)
    expect(first.revision).toBe(1)
    expect(first.loaded).toEqual(['code_crystal'])
    expect(mgr.getRevision()).toBe(1)
    expect(mgr.getStatus()).toBe('ready')
    expect(events).toHaveLength(1)
    expect(events[0].revision).toBe(1)
    expect(events[0].kinds).toEqual(['code_crystal'])

    const second = await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh())))
    expect(second.revision).toBe(2)
    expect(mgr.getRevision()).toBe(2)
    expect(events).toHaveLength(2)
    expect(events[1].revision).toBe(2)
  })

  it('全部失败：revision 不变、getAsset 为 null、状态 failed、错误透传', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './missing.glb')

    const res = await mgr.loadPropAssets(failingLoader('HTTP 404'))

    expect(res.ok).toBe(false)
    expect(res.revision).toBeNull()
    expect(res.loaded).toEqual([])
    expect(mgr.getRevision()).toBe(0)
    expect(mgr.getAsset('code_crystal')).toBeNull()
    expect(mgr.getStatus()).toBe('failed')
    expect(res.errors).toHaveLength(1)
    expect(res.errors[0].code).toBe('load_failed')
    expect(res.errors[0].path).toBe('code_crystal')
    expect(mgr.getLastErrors()).toEqual(res.errors)
  })

  it('失败条目保持旧模板：先成功装 A，再让 A 失败装 B，A 仍在且未被替换', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './a.glb')

    const sceneA = sceneWith(boxMesh())
    await mgr.loadPropAssets(fakeLoader(() => sceneA))
    expect(mgr.getAsset('code_crystal')).toBe(sceneA)
    expect(mgr.getRevision()).toBe(1)

    const sceneB = sceneWith(boxMesh())
    mgr.registerPropAsset('doc_scroll', './b.glb')
    const res = await mgr.loadPropAssets(
      fakeLoader((url) => {
        if (url === './a.glb') throw new Error('A 这一轮坏了')
        return sceneB
      }),
    )

    // B 成功入库 → 整体仍算成功（≥1 条目）；A 是必选失败 → 进 errors，但旧模板保留
    expect(res.ok).toBe(true)
    expect(res.revision).toBe(2)
    expect(res.loaded).toEqual(['doc_scroll'])
    expect(res.errors.some((e) => e.path === 'code_crystal' && e.code === 'load_failed')).toBe(true)
    expect(mgr.getAsset('code_crystal')).toBe(sceneA)
    expect(mgr.getAsset('doc_scroll')).toBe(sceneB)
    expect(mgr.getRevision()).toBe(2)
  })

  it('optional 资产加载失败只进 warnings，不算阻断 errors', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './missing.glb', { optional: true })

    const res = await mgr.loadPropAssets(failingLoader('HTTP 404'))

    expect(res.errors).toEqual([])
    expect(res.warnings).toHaveLength(1)
    expect(res.warnings[0].level).toBe('warning')
    expect(res.warnings[0].code).toBe('load_failed')
    expect(res.warnings[0].path).toBe('code_crystal')
    // 无成功条目 → 状态 failed，但没有任何阻断错误
    expect(mgr.getStatus()).toBe('failed')
    expect(mgr.getAsset('code_crystal')).toBeNull()
  })

  it('reset() 清空模板 / revision / 状态 / 错误', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './a.glb')
    await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh())))
    expect(mgr.getRevision()).toBe(1)
    expect(mgr.getAsset('code_crystal')).not.toBeNull()

    mgr.reset()

    expect(mgr.getAsset('code_crystal')).toBeNull()
    expect(mgr.getRevision()).toBe(0)
    expect(mgr.getStatus()).toBe('idle')
    expect(mgr.getLastErrors()).toEqual([])
  })

  it('registerPropAsset 同 kind 覆盖前次登记（只加载最新 url）', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './old.glb')
    mgr.registerPropAsset('code_crystal', './new.glb')

    const seen: string[] = []
    const res = await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh()), (url) => seen.push(url)))

    expect(seen).toEqual(['./new.glb'])
    expect(res.ok).toBe(true)
    expect(res.loaded).toEqual(['code_crystal'])
    expect(res.revision).toBe(1)
  })

  it('取消订阅后不再收到通知（但资产照常替换、revision 照常递增）', async () => {
    const mgr = new GlbHotReloadManager()
    mgr.registerPropAsset('code_crystal', './a.glb')
    const spy = vi.fn()
    const off = mgr.subscribe(spy)

    await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh())))
    expect(spy).toHaveBeenCalledTimes(1)

    off()
    await mgr.loadPropAssets(fakeLoader(() => sceneWith(boxMesh())))
    expect(spy).toHaveBeenCalledTimes(1)
    expect(mgr.getRevision()).toBe(2)
  })
})
