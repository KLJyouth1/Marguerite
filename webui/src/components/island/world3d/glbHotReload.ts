/**
 * GLB 热加载管理器（Blender 资产管线 → 3D 场景的消费端接线）。
 *
 * ── 闭环定位 ──────────────────────────────────────────────────────────
 * blender/asset_pipeline.py 把「AI 资产规格 JSON」建模导出成 GLB；本模块在
 * 运行时用 Three.js GLTFLoader 把 GLB 校验后热加载进场景（scene.ts 消费），
 * 补齐「spec JSON → Blender 建模导出 GLB → 校验 → 运行时热加载」闭环。
 *
 * ── 与 gddHotReload 同构的「原子替换 + 失败回退」语义 ────────────────
 * - **逐条目校验、整批原子替换**：loadPropAssets 并行取全部登记资产，
 *   每条都过校验（可解析 / 含 Mesh / 包围盒非退化 / 面数预算）后才整体替换
 *   当前模板表；失败的条目保持旧模板（或回退程序化几何），绝不半更新；
 * - **revision 递增**：每次成功替换 revision+1，订阅方按版本增量热替换实例；
 * - **错误透传**：失败明细（GlbIssue）原样交给 UI（islandGame.glb.* 词条），
 *   「校验失败」是可见的产品状态而非控制台噪声；
 * - **状态机**：idle → loading → ready/failed（failed 不覆盖旧资产，只是
 *   报告本轮整体状态，让 UI 知道「该轮热加载没换上新资产」）。
 *
 * ── 为什么面数预算（默认 ≤ 50k 三角形）────────────────────────────────
 * AI 产出是「失控高发区」：规格里一个 LOD 参数写错、导出时细分级别拉满，
 * 就能产出几百万面的资产，直接拖垮中低端机渲染与内存。面数预算是在校验
 * 入口的硬闸门——超预算的 GLB 拒绝入库，回退程序化几何，宁可「不升级」
 * 也不能让失控资产毁掉整个场景。
 *
 * ── 为什么程序化几何是 fallback 而非删除 ──────────────────────────────
 * 程序化几何（scene.ts 的 mainGeometry/accentGeometry）是资产的**可选升级
 * 出口**与**无素材依赖兜底**：GLB 不存在/加载失败/校验被拒时，道具照常以
 * 程序化几何渲染，世界永不塌；有了 GLB 才升级视觉。Blender 管线是升级
 * 出口而非依赖（terrain.ts 文件头同一论点）。
 *
 * 订阅是普通 Set 回调（不绑 vue 响应式）：契约层保持纯 TS，消费方
 * （scene.ts / IslandWorld3D.vue）自行桥接。
 */

import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import type { PropKind } from './propsSpec'

// ───────────────────────── 契约类型 ─────────────────────────

/** GLB 加载状态机（UI 可显式展示「加载中/就绪/失败」） */
export type GlbLoadStatus = 'idle' | 'loading' | 'ready' | 'failed'

/** 校验/加载错误明细（结构与 gddSchema.GddIssue 同构：level/code/path/message） */
export interface GlbIssue {
  /** error 阻断该条目入库（回退程序化几何）；warning 只提示不阻断 */
  level: 'error' | 'warning'
  /** 机器可读错误码（UI/测试断言用） */
  code: string
  /** 出错位置（道具种类，如 code_crystal） */
  path: string
  /** 人类可读明细（语言中立描述，i18n 标签由 UI 决定） */
  message: string
}

/** 一次 loadPropAssets 的整体结果（成功/失败都带完整上下文） */
export interface GlbLoadResult {
  ok: boolean
  /** 成功替换后的生效 revision（失败为 null） */
  revision: number | null
  /** 本轮成功的道具种类（已原子替换） */
  loaded: PropKind[]
  /** 阻断性错误（按条目透传；ok=false 时非空） */
  errors: GlbIssue[]
  /** 非阻断提示 */
  warnings: GlbIssue[]
}

/** 变更通知：只在成功替换后触发（失败保持旧资产，不打扰订阅方） */
export interface GlbChangeEvent {
  revision: number
  /** 本次被替换的道具种类（订阅方可按需热替换实例） */
  kinds: PropKind[]
  warnings: GlbIssue[]
}

export type GlbListener = (event: GlbChangeEvent) => void

/** 资产登记（manifest.json 的运行时形态） */
export interface GlbAssetEntry {
  kind: PropKind
  /** GLB 相对 URL（相对 webui 静态资产根；见 manifest.json 的资产目录说明） */
  url: string
  /** 可选资产：加载失败静默回退程序化几何（本机无 Blender 时的诚实声明） */
  optional?: boolean
  /** 来源（procedural/Blender 导出/第三方 URL）——许可红线的可审计字段 */
  source?: string
  /** 许可（project/cc0/…）——来源不明、AI 抓取、CC-BY 未署名的模型禁止入库 */
  license?: string
}

/**
 * 面数预算（三角形数上限）。为什么需要：AI 产出是失控高发区——细分级别/
 * LOD 参数写错就能导出百万面资产，拖垮中低端机渲染与内存。超预算 = 拒绝
 * 入库回退程序化几何，宁可「不升级」也不让失控资产毁掉场景。
 */
export const DEFAULT_TRIANGLE_BUDGET = 50_000

/** 校验选项（测试可注入更小预算逐条断言） */
export interface GlbValidateOptions {
  /** 三角形预算上限（默认 DEFAULT_TRIANGLE_BUDGET） */
  triangleBudget?: number
}

/** 校验结果：通过时带 GLB 场景模板（未克隆的原对象，入库前不暴露给消费方） */
interface GlbValidation {
  ok: boolean
  errors: GlbIssue[]
  warnings: GlbIssue[]
  template: THREE.Object3D | null
}

// ───────────────────────── GLB 校验（纯函数，可 vitest）─────────────────────────

/**
 * 校验一份 GLB 场景：
 * 1. 至少含 1 个 Mesh（空场景/纯相机灯光的 GLB 没有可渲染网格）；
 * 2. 包围盒非退化（尺寸 > 0——零体积资产是导出事故的典型症状）；
 * 3. 面数预算（默认 ≤ 50k 三角形，防 AI 产出失控资产）。
 *
 * 通过时返回模板对象（调用方负责入库；消费方必须 clone 后使用，
 * 共享几何/材质，不共享变换）。
 */
export function validateGlbScene(
  scene: THREE.Object3D,
  kind: string,
  options: GlbValidateOptions = {},
): GlbValidation {
  const errors: GlbIssue[] = []
  const warnings: GlbIssue[] = []
  const budget = options.triangleBudget ?? DEFAULT_TRIANGLE_BUDGET

  // 1. 至少 1 个 Mesh + 2. 逐网格面数清点
  let meshCount = 0
  let triangles = 0
  scene.traverse((o) => {
    const mesh = o as THREE.Mesh
    if (!mesh.isMesh) return
    meshCount += 1
    triangles += countTriangles(mesh.geometry)
  })

  if (meshCount === 0) {
    errors.push({
      level: 'error',
      code: 'no_mesh',
      path: kind,
      message: 'GLB 场景不含任何 Mesh（空场景不可渲染）',
    })
    return { ok: false, errors, warnings, template: null }
  }

  // 3. 面数预算（AI 产出失控防护）
  if (triangles > budget) {
    errors.push({
      level: 'error',
      code: 'triangle_budget_exceeded',
      path: kind,
      message: `GLB 面数 ${triangles} 超出预算 ${budget}（AI 产出失控防护，回退程序化几何）`,
    })
    return { ok: false, errors, warnings, template: null }
  }

  // 2. 包围盒非退化（尺寸 > 0）
  const box = new THREE.Box3().setFromObject(scene)
  if (box.isEmpty()) {
    errors.push({
      level: 'error',
      code: 'degenerate_bounds',
      path: kind,
      message: 'GLB 包围盒为空（导出事故，回退程序化几何）',
    })
    return { ok: false, errors, warnings, template: null }
  }
  const size = box.getSize(new THREE.Vector3())
  if (!(size.x > 0) || !(size.y > 0) || !(size.z > 0)) {
    errors.push({
      level: 'error',
      code: 'degenerate_bounds',
      path: kind,
      message: `GLB 包围盒退化（尺寸 ${size.x}×${size.y}×${size.z}，至少一轴为 0）`,
    })
    return { ok: false, errors, warnings, template: null }
  }

  return { ok: true, errors, warnings, template: scene }
}

/** 清点一个几何体的三角形数（索引几何按索引数，非索引几何按顶点数） */
function countTriangles(geometry: THREE.BufferGeometry): number {
  const index = geometry.index
  if (index) return Math.floor(index.count / 3)
  const position = geometry.getAttribute('position')
  return position ? Math.floor(position.count / 3) : 0
}

// ───────────────────────── 热加载管理器 ─────────────────────────

/**
 * GLB 热加载管理器。可 new 多个实例（测试/多世界），也可用模块级
 * glbHotReload 单例接 scene。
 */
export class GlbHotReloadManager {
  /** kind → 已校验模板（原子替换的唯一真相源） */
  private templates = new Map<PropKind, THREE.Object3D>()
  private revision = 0
  private listeners = new Set<GlbListener>()
  /** 状态机：idle → loading → ready/failed */
  private status: GlbLoadStatus = 'idle'
  /** 最近一次失败的错误明细（UI 常驻显示直到下次成功） */
  private lastErrors: GlbIssue[] = []
  private registered: GlbAssetEntry[] = []
  private options: GlbValidateOptions

  constructor(options: GlbValidateOptions = {}) {
    this.options = options
  }

  /**
   * 登记道具种类 → GLB URL（可多次调用；同 kind 后登记覆盖前登记）。
   * 登记本身不触发加载——loadPropAssets 才并行取资产。
   */
  registerPropAsset(kind: PropKind, url: string, meta: Omit<GlbAssetEntry, 'kind' | 'url'> = {}): void {
    this.registered = this.registered.filter((e) => e.kind !== kind)
    this.registered.push({ kind, url, ...meta })
  }

  /** 批量登记（manifest.json 的运行时入口） */
  registerAll(entries: GlbAssetEntry[]): void {
    for (const e of entries) this.registerPropAsset(e.kind, e.url, e)
  }

  /**
   * 并行加载全部登记的 GLB，逐条校验后**原子替换**。
   *
   * 语义（与 gddHotReload.loadFromJson 同构）：
   * - 每条目独立校验；校验通过的条目进入「候选表」，失败的条目保持旧
   *   模板（首次加载失败 → 无模板 → scene 回退程序化几何）；
   * - 至少 1 条目成功才原子替换候选表 + revision 递增 + 通知订阅方；
   *   全部失败 → 旧模板一个字节不动，状态机置 failed，错误透传；
   * - optional 资产（本机无 Blender 时 GLB 尚不存在）加载失败静默回退，
   *   不算阻断错误——诚实性红线：不假装资产存在。
   */
  async loadPropAssets(loader: Pick<GLTFLoader, 'loadAsync'> = new GLTFLoader()): Promise<GlbLoadResult> {
    this.status = 'loading'
    const errors: GlbIssue[] = []
    const warnings: GlbIssue[] = []
    const candidates = new Map<PropKind, THREE.Object3D>()
    const loadedKinds: PropKind[] = []

    const settled = await Promise.all(
      this.registered.map(async (entry) => {
        try {
          const gltf = await loader.loadAsync(entry.url)
          const validation = validateGlbScene(gltf.scene, entry.kind, this.options)
          return { entry, validation, loadError: null }
        } catch (err) {
          return {
            entry,
            validation: null,
            loadError: err instanceof Error ? err.message : String(err),
          }
        }
      }),
    )

    for (const { entry, validation, loadError } of settled) {
      if (loadError !== null) {
        // 加载失败：optional 资产静默回退（诚实声明「资产尚不存在」）；
        // 必选资产是阻断错误，但只影响该条目——其余条目照常入库。
        const issue: GlbIssue = {
          level: entry.optional ? 'warning' : 'error',
          code: 'load_failed',
          path: entry.kind,
          message: `GLB 加载失败：${loadError}`,
        }
        ;(entry.optional ? warnings : errors).push(issue)
        continue
      }
      const result = validation as GlbValidation
      if (!result.ok || !result.template) {
        errors.push(...result.errors)
        warnings.push(...result.warnings)
        continue
      }
      warnings.push(...result.warnings)
      candidates.set(entry.kind, result.template)
      loadedKinds.push(entry.kind)
    }

    // 至少 1 条目成功才原子替换（全失败 → 旧模板保持，状态机 failed）
    if (loadedKinds.length === 0) {
      this.status = 'failed'
      this.lastErrors = errors
      return { ok: false, revision: null, loaded: [], errors, warnings }
    }

    // 原子替换：新表整体生效（旧模板被替换掉的条目由消费方热替换时 dispose）
    for (const [kind, template] of candidates) this.templates.set(kind, template)
    this.revision += 1
    this.status = 'ready'
    this.lastErrors = []
    const event: GlbChangeEvent = { revision: this.revision, kinds: loadedKinds, warnings }
    for (const fn of this.listeners) fn(event)
    return { ok: true, revision: this.revision, loaded: loadedKinds, errors, warnings }
  }

  /**
   * 取实例模板（供 scene clone 实例化，共享几何/材质）。
   * 未加载/该条目失败 → null（消费方回退程序化几何）。
   */
  getAsset(kind: PropKind): THREE.Object3D | null {
    return this.templates.get(kind) ?? null
  }

  /** 当前生效版本号（0 = 尚未成功加载） */
  getRevision(): number {
    return this.revision
  }

  /** 状态机当前态 */
  getStatus(): GlbLoadStatus {
    return this.status
  }

  /** 最近一次失败的错误明细（成功加载后清空） */
  getLastErrors(): GlbIssue[] {
    return this.lastErrors
  }

  /**
   * 订阅变更（成功替换时通知）。返回取消订阅函数。
   * 只通知成功的替换——失败时资产没变，订阅方无需重绘。
   */
  subscribe(fn: GlbListener): () => void {
    this.listeners.add(fn)
    return () => {
      this.listeners.delete(fn)
    }
  }

  /** 清空全部模板（测试/世界切换用；消费方负责 dispose 已克隆实例） */
  reset(): void {
    this.templates.clear()
    this.revision = 0
    this.status = 'idle'
    this.lastErrors = []
  }
}

/** 模块级单例（scene/IslandWorld3D 消费方共用一份 GLB 资产表） */
export const glbHotReload = new GlbHotReloadManager()
