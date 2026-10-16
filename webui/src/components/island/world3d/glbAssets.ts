/**
 * GLB 资产接线（manifest.json → glbHotReload 消费端的最后一公里）。
 *
 * ── 闭环定位 ──────────────────────────────────────────────────────────
 * 本模块打通「AI 资产规格 JSON → Blender 导出 GLB → 运行时校验 → 场景热替换」
 * 闭环的**登记/加载端**：
 *   1. resolveGlbUrl 把 manifest 里的相对登记名解析成可 fetch 的真实 URL；
 *   2. registerIslandGlbAssets 把 5 种道具登记进 glbHotReload（不触发加载）；
 *   3. loadIslandGlbAssets 触发并行加载 + 校验 + 原子替换（glbHotReload 语义），
 *      返回 GlbLoadResult 供 UI 显示「程序化几何 / 已热加载 N 件 / 校验失败」。
 * scene.ts 订阅 glbHotReload 的变更事件，在加载成功后热替换场景里的道具几何。
 *
 * ── 诚实性红线（为什么加载失败绝不能抛）────────────────────────────────
 * 本机未安装 Blender（`blender --version` 不存在），5 种 GLB 尚未导出，manifest
 * 里全部登记为 optional: true。**GLB 不存在是预期状态**：加载失败必须静默回退
 * 程序化几何，绝不抛异常阻断渲染，也绝不假装「AI 已产出资产」。因此
 * loadIslandGlbAssets 只透传 glbHotReload 的 GlbLoadResult（ok=false 是正常
 * 状态之一），UI 据此如实显示「程序化几何」而非「已加载」。
 *
 * ── 为什么 URL 解析与 useBgm.musicAssetUrl 同构 ────────────────────────
 * GLB 走 webui/public/ 静态资产目录（与 music/、pets/ 同模式），Vite 原样复制
 * 进 dist：DEV 直接取 /island3d/<file>；PROD 由网关从 <basePath>/static/dist/
 * 提供（basePath 取 #Marguerite-data 的 dataset.basePath）。两处保持一致，
 * 避免出现「音乐能放、模型 404」这类环境差异坑。
 */

import manifestJson from './assets/manifest.json'
import {
  glbHotReload,
  type GlbAssetEntry,
  type GlbHotReloadManager,
  type GlbLoadResult,
} from './glbHotReload'
import type { PropKind } from './propsSpec'
import type { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'

// ───────────────────────── manifest 契约 ─────────────────────────

/** manifest.json 的单条资产（只声明消费端实际读到的字段） */
export interface IslandGlbManifestAsset {
  kind: PropKind
  /** 相对资产根的登记名（如 ./code_crystal.glb） */
  url: string
  /** 可选资产：加载失败静默回退程序化几何 */
  optional?: boolean
  /** 许可红线：来源不明/AI 抓取/CC-BY 未署名模型禁止入库 */
  license?: string
  /** 来源可审计字段 */
  source?: string
}

/** manifest.json 的运行时形态 */
export interface IslandGlbManifest {
  version: number
  assets: IslandGlbManifestAsset[]
}

/**
 * 静态清单（Vite 原生支持 JSON import；tsconfig 已开 resolveJsonModule）。
 * JSON 里的 `kind` 被推断成 string，故经 unknown 收窄到 PropKind 联合。
 */
export const ISLAND_GLB_MANIFEST = manifestJson as unknown as IslandGlbManifest

/** 资产静态根目录（相对 webui 静态资产根；与 music/、pets/ 同模式） */
const ISLAND_GLB_DIR = 'island3d'

/** PROD 下无 #Marguerite-data 时的兜底 basePath（与 useBgm.musicAssetUrl 一致） */
const DEFAULT_BASE_PATH = '/control'

// ───────────────────────── URL 解析 ─────────────────────────

/**
 * 把 manifest 的登记名（`./<kind>.glb` 或裸名）解析成可 fetch 的真实 URL。
 *
 * - DEV：Vite 把 public/ 挂在站点根 → `/island3d/<file>`；
 * - PROD：打包 UI 由网关从 `<basePath>/static/dist/island3d/<file>` 提供，
 *   basePath 取 #Marguerite-data 的 dataset.basePath（缺省 `/control`）；
 * - node 测试环境无 document → 安全回退默认 basePath，绝不因缺 DOM 崩溃。
 */
export function resolveGlbUrl(relOrName: string): string {
  // 只取文件名、忽略目录前缀：保证 DEV/PROD 两形态都得到稳定的
  // /island3d/<file>，登记名写法（./x、x、sub/x）不影响结果。
  const file = relOrName.replace(/^\.\//, '').split('/').pop() || relOrName
  if (import.meta.env.DEV) return `/${ISLAND_GLB_DIR}/${file}`
  const base =
    typeof document !== 'undefined'
      ? document.getElementById('Marguerite-data')?.dataset.basePath || DEFAULT_BASE_PATH
      : DEFAULT_BASE_PATH
  return `${base.replace(/\/$/, '')}/static/dist/${ISLAND_GLB_DIR}/${file}`
}

// ───────────────────────── 登记 + 加载 ─────────────────────────

/**
 * 把 manifest 的资产经 URL 解析后**登记**进管理器（不触发加载）。
 * 可重复调用（同 kind 后登记覆盖前登记）；默认接模块级单例。
 */
export function registerIslandGlbAssets(manager: GlbHotReloadManager = glbHotReload): void {
  manager.registerAll(
    ISLAND_GLB_MANIFEST.assets.map(
      (asset): GlbAssetEntry => ({
        kind: asset.kind,
        url: resolveGlbUrl(asset.url),
        optional: asset.optional,
        source: asset.source,
        license: asset.license,
      }),
    ),
  )
}

/**
 * 登记 manifest 全部资产并触发加载（glbHotReload 的原子替换 + 失败回退语义）。
 *
 * 全部资产 optional → 加载失败只产生 warning，返回 ok=false 但**绝不抛异常**。
 * 返回 GlbLoadResult 供 UI 如实显示（loaded 数量 / revision / 错误明细）。
 * loader 可注入（测试用 fake loader；缺省走 GLTFLoader）。
 */
export function loadIslandGlbAssets(
  loader?: Pick<GLTFLoader, 'loadAsync'>,
  manager: GlbHotReloadManager = glbHotReload,
): Promise<GlbLoadResult> {
  registerIslandGlbAssets(manager)
  // loadPropAssets 内部已 try/catch 每条目加载：全 optional 时失败仅 warning。
  // 诚实性红线：这里不吞掉结果、不伪造成功——UI 依据 ok/loaded 如实呈现。
  return manager.loadPropAssets(loader)
}
