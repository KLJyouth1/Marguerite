/**
 * glbAssets 接线单测（node 环境，无 jsdom）。
 *
 * 覆盖：URL 解析（DEV/PROD/basePath 回退）、manifest 登记、全 optional 失败不抛、
 * 成功加载后 getAsset/revision、reset 归零。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { existsSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
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

/**
 * manifest 与磁盘的一致性。
 *
 * 为什么需要这组断言（2026-10-21 逐页巡检发现）：`/island-game` 每次加载都打
 * 4 个 `.glb` 的 404，而当时所有门禁全绿——因为原测试只验证
 * `resolveGlbUrl()` 的**字符串拼接**，从不检查文件是否存在。于是
 * "manifest 登记 5 件、磁盘上只有 1 件"这件事没有任何地方能发现。
 *
 * 判定分三层，各拦一类错误：
 *   1. 非 optional 资产 → 文件**必须**存在（缺了就是 404，必须门禁红）；
 *   2. optional 资产若存在 → 必须非空（0 字节是"假装导出成功"）；
 *   3. optional 且缺失 → 属"资产尚未产出"，不判红，但必须打印清单：
 *      这条信息是"未产出"的可见痕迹，避免它悄悄变成用户眼里的 404。
 */
describe('manifest ↔ 磁盘一致性', () => {
  const island3dDir = fileURLToPath(new URL('../../../../public/island3d/', import.meta.url))

  function fileNameOf(relOrName: string): string {
    return relOrName.replace(/^\.\//, '').split('/').pop() || relOrName
  }

  it('非 optional 资产必须真实存在于 public/island3d', () => {
    const required = ISLAND_GLB_MANIFEST.assets.filter((a) => a.optional !== true)
    const missing = required
      .map((a) => fileNameOf(a.url))
      .filter((file) => !existsSync(`${island3dDir}${file}`))
    expect(
      missing,
      `manifest 把以下资产声明为必需，但 public/island3d 下没有对应文件（会 404）：${missing.join(', ')}`,
    ).toEqual([])
  })

  it('已存在的资产文件必须非空（0 字节 = 伪装成功）', () => {
    const empties: string[] = []
    for (const asset of ISLAND_GLB_MANIFEST.assets) {
      const path = `${island3dDir}${fileNameOf(asset.url)}`
      if (!existsSync(path)) continue
      if (statSync(path).size === 0) empties.push(path)
    }
    expect(empties).toEqual([])
  })

  it('把「尚未导出」的资产显式列出来（诚实痕迹，不静默）', () => {
    const pending = ISLAND_GLB_MANIFEST.assets
      .filter((a) => a.optional === true)
      .map((a) => fileNameOf(a.url))
      .filter((file) => !existsSync(`${island3dDir}${file}`))
    // 不把"未导出"判成失败：本机确实没装 Blender（见 glbAssets.ts 文件头），
    // 这是被 manifest 显式声明的预期状态。但如果清单**为空**却仍有 optional
    // 标记，说明 optional 已经名不副实，应清理标记而不是继续挂着。
    if (pending.length === 0) {
      const stillOptional = ISLAND_GLB_MANIFEST.assets.filter((a) => a.optional === true)
      expect(
        stillOptional,
        '所有资产都已落地，仍标 optional 的条目应移除标记（否则 future 缺失会被静默放过）',
      ).toEqual([])
    }
    // 可见性：让"待导出 N 件"出现在测试输出里，而不是只在浏览器控制台。
    console.log(`[glbAssets] 待导出资产 ${pending.length} 件：${pending.join(', ') || '（无）'}`)
    expect(Array.isArray(pending)).toBe(true)
  })
})

describe('resolveGlbUrl', () => {
  it('DEV：解析为站点根下的 /island3d/<file>', () => {
    vi.stubEnv('DEV', true)
    expect(resolveGlbUrl('./code_crystal.glb')).toBe('/island3d/code_crystal.glb')
    expect(resolveGlbUrl('data_fruit.glb')).toBe('/island3d/data_fruit.glb')
    // 目录前缀被忽略，只取文件名
    expect(resolveGlbUrl('./nested/quest_beacon.glb')).toBe('/island3d/quest_beacon.glb')
  })

  it('PROD：解析为 <basePath>/static/island3d/<file>（node 无 document → 回退 /control）', () => {
    vi.stubEnv('DEV', false)
    // 回归：网关 /static/ 已映射 dist 根；多拼一层 dist 会命中 SPA 回落（text/html）。
    expect(resolveGlbUrl('./code_crystal.glb')).toBe(
      '/control/static/island3d/code_crystal.glb',
    )
  })

  it('PROD：document 存在但缺 basePath 回退 /control；有 basePath 则按其值解析', () => {
    vi.stubEnv('DEV', false)

    vi.stubGlobal('document', {
      getElementById: () => ({ dataset: {} }),
    } as unknown as Document)
    expect(resolveGlbUrl('./doc_scroll.glb')).toBe(
      '/control/static/island3d/doc_scroll.glb',
    )

    vi.stubGlobal('document', {
      getElementById: (id: string) =>
        id === 'Marguerite-data' ? { dataset: { basePath: '/custom/' } } : null,
    } as unknown as Document)
    // 末尾斜杠被归一化，不产生双斜杠
    expect(resolveGlbUrl('./doc_scroll.glb')).toBe(
      '/custom/static/island3d/doc_scroll.glb',
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
