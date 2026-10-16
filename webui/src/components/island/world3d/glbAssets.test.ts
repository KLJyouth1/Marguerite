/**
 * glbAssets 接线单测（node 环境，无 jsdom）。
 *
 * 覆盖：URL 解析（DEV/PROD/basePath 回退）、manifest 登记、全 optional 失败不抛、
 * 成功加载后 getAsset/revision、reset 归零。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import type { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import {
  ISLAND_GLB_MANIFEST,
  loadIslandGlbAssets,
  registerIslandGlbAssets,
  resolveGlbUrl,
} from './glbAssets'
import { GlbHotReloadManager } from './glbHotReload'

/** 构造一个可通过校验的最小 GLB 场景（1 个非退化 Mesh） */
function validGlbScene(): THREE.Object3D {
  const group = new THREE.Group()
  group.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial()))
  return group
}

/** fake loader：记录被请求的 URL，可注入成功/失败行为 */
function fakeLoader(resolveScene: (url: string) => THREE.Object3D = () => validGlbScene()) {
  const urls: string[] = []
  const loader = {
    loadAsync: async (url: string) => {
      urls.push(url)
      return { scene: resolveScene(url) }
    },
  }
  return { loader: loader as unknown as Pick<GLTFLoader, 'loadAsync'>, urls }
}

/** 模拟「本机无 Blender → GLB 不存在」的加载失败 loader */
function throwingLoader(message = 'ENOENT: GLB 不存在（本机无 Blender）') {
  const loader = {
    loadAsync: async (_url: string): Promise<never> => {
      throw new Error(message)
    },
  }
  return loader as unknown as Pick<GLTFLoader, 'loadAsync'>
}

afterEach(() => {
  vi.unstubAllEnvs()
  vi.unstubAllGlobals()
})

describe('resolveGlbUrl', () => {
  it('DEV：解析为站点根下的 /island3d/<file>', () => {
    vi.stubEnv('DEV', true)
    expect(resolveGlbUrl('./code_crystal.glb')).toBe('/island3d/code_crystal.glb')
    expect(resolveGlbUrl('data_fruit.glb')).toBe('/island3d/data_fruit.glb')
    // 目录前缀被忽略，只取文件名
    expect(resolveGlbUrl('./nested/quest_beacon.glb')).toBe('/island3d/quest_beacon.glb')
  })

  it('PROD：解析为 <basePath>/static/dist/island3d/<file>（node 无 document → 回退 /control）', () => {
    vi.stubEnv('DEV', false)
    expect(resolveGlbUrl('./code_crystal.glb')).toBe(
      '/control/static/dist/island3d/code_crystal.glb',
    )
  })

  it('PROD：document 存在但缺 basePath 回退 /control；有 basePath 则按其值解析', () => {
    vi.stubEnv('DEV', false)

    vi.stubGlobal('document', {
      getElementById: () => ({ dataset: {} }),
    } as unknown as Document)
    expect(resolveGlbUrl('./doc_scroll.glb')).toBe(
      '/control/static/dist/island3d/doc_scroll.glb',
    )

    vi.stubGlobal('document', {
      getElementById: (id: string) =>
        id === 'Marguerite-data' ? { dataset: { basePath: '/custom/' } } : null,
    } as unknown as Document)
    // 末尾斜杠被归一化，不产生双斜杠
    expect(resolveGlbUrl('./doc_scroll.glb')).toBe(
      '/custom/static/dist/island3d/doc_scroll.glb',
    )
  })
})

describe('registerIslandGlbAssets', () => {
  it('登记 manifest 全部 5 件，URL 按环境解析（经 fake loader 观测）', async () => {
    vi.stubEnv('DEV', true)
    const mgr = new GlbHotReloadManager()
    registerIslandGlbAssets(mgr)

    const { loader, urls } = fakeLoader()
    const result = await mgr.loadPropAssets(loader)

    expect(ISLAND_GLB_MANIFEST.assets).toHaveLength(5)
    expect(urls).toHaveLength(5)
    expect(new Set(urls).size).toBe(5)
    expect(urls.every((u) => u.startsWith('/island3d/'))).toBe(true)
    expect(result.loaded).toHaveLength(5)
  })
})

describe('loadIslandGlbAssets', () => {
  it('全 optional 加载失败：不抛异常，ok=false 且全部落在 warnings（诚实回退）', async () => {
    vi.stubEnv('DEV', true)
    const mgr = new GlbHotReloadManager()

    const result = await loadIslandGlbAssets(throwingLoader(), mgr)

    expect(result.ok).toBe(false)
    expect(result.revision).toBeNull()
    // 全部 optional → 无阻断错误，只透传 warning
    expect(result.errors).toHaveLength(0)
    expect(result.warnings).toHaveLength(ISLAND_GLB_MANIFEST.assets.length)
    expect(result.warnings.every((w) => w.code === 'load_failed')).toBe(true)
    // 未入库 → 消费方回退程序化几何
    expect(mgr.getAsset('code_crystal')).toBeNull()
  })

  it('成功加载后 getAsset 非 null 且 revision 递增', async () => {
    vi.stubEnv('DEV', true)
    const mgr = new GlbHotReloadManager()
    expect(mgr.getRevision()).toBe(0)

    const { loader } = fakeLoader()
    const r1 = await loadIslandGlbAssets(loader, mgr)
    expect(r1.ok).toBe(true)
    expect(r1.revision).toBe(1)
    expect(mgr.getRevision()).toBe(1)
    expect(mgr.getAsset('code_crystal')).not.toBeNull()

    const r2 = await loadIslandGlbAssets(loader, mgr)
    expect(r2.revision).toBe(2)
    expect(mgr.getRevision()).toBe(2)
    expect(mgr.getAsset('doc_scroll')).not.toBeNull()
  })

  it('reset 后 getAsset 归 null、revision 归 0', async () => {
    vi.stubEnv('DEV', true)
    const mgr = new GlbHotReloadManager()
    await loadIslandGlbAssets(fakeLoader().loader, mgr)
    expect(mgr.getAsset('code_crystal')).not.toBeNull()

    mgr.reset()
    expect(mgr.getAsset('code_crystal')).toBeNull()
    expect(mgr.getRevision()).toBe(0)
    expect(mgr.getStatus()).toBe('idle')
  })
})
