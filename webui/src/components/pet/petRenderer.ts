/**
 * 桌宠渲染器工厂：**优先真三维（three.js）**，不可用或构造失败时如实回退 2.5D 矢量。
 *
 * 为什么单独成文件：页内桌宠（`DesktopPet`）与桌面悬浮窗（`PetOverlayView`）需要
 * 同一套「先 3D、失败再降级」的判据。两处各写一份必然漂移 —— 真机上出现过的现象
 * 就是「悬浮窗里的桌宠不是 3D」，而页内那张是。`PetView` 例外：它把渲染模式做成
 * 用户可切换的开关（2.5D / 3D），因此自己直接构造两种渲染器。
 *
 * 3D 运行时（three）走**动态 import**：`DesktopPet` 挂在 App 外壳上，静态引入会把
 * 整个 three 塞进首屏包（主题包守卫与首屏体积都会破）。
 *
 * 降级是**诚实**的：三维不可用时返回 `kind: 'svg'` 并带上 `fallbackReason`，
 * 不留下空白舞台，也不谎称已渲染 3D。
 */
import type { PetMood3D, PetRenderTier } from './petContract'
import type { PetModelSpec } from './petSpec'
import type { PetHitRegion } from './petPose'
import { PetSvg } from './petSvg'
import { isWebGLAvailable } from './pet3d/webglSupport'
import type { Pet3DView } from './pet3d/views'

/** 两种渲染器的公共子集 —— 宿主只依赖这组方法，切换渲染器零改动。 */
export interface PetRenderer {
  setMood(mood: PetMood3D): void
  triggerMoodAction(mood: PetMood3D): void
  setFacing(dir: 1 | -1): void
  setHover(weight: number): void
  lookAt(x: number, y: number): void
  /** 部位轻点（头 / 背 / 尾） */
  poke(region: PetHitRegion): void
  jump(strength?: number): void
  burst(color?: number, count?: number): void
  spin(duration?: number): void
  dance(duration?: number): void
  setWalking(walking: boolean): void
  dispose(): void
}

export interface CreatePetRendererOptions {
  container: HTMLElement
  spec: PetModelSpec
  accent?: number
  /** 渲染档位：只影响特效预算（粒子/阴影），不影响形象完整度 */
  quality?: PetRenderTier
  /** 是否允许三维（缺省 true）；false 直接走矢量 */
  prefer3d?: boolean
  /** 三维初始视角，缺省 `front`（桌面桌宠不该一上来就侧着） */
  view?: Pet3DView
}

export interface PetRendererResult {
  renderer: PetRenderer
  kind: '3d' | 'svg'
  /** 回退原因（仅「本可以 3D 却失败」时给出），供调用方如实展示 */
  fallbackReason?: string
}

export async function createPetRenderer(
  opts: CreatePetRendererOptions,
): Promise<PetRendererResult> {
  const svg = (): PetSvg => {
    // 容器里可能残留失败的三维 canvas：清掉再挂 SVG，避免两层叠在一起
    opts.container.replaceChildren()
    return new PetSvg({
      container: opts.container,
      accent: opts.accent,
      spec: opts.spec,
      quality: opts.quality,
    })
  }

  if (opts.prefer3d !== false && isWebGLAvailable()) {
    try {
      const mod = await import('./pet3d/Pet3DScene')
      const scene = new mod.Pet3DScene({
        container: opts.container,
        accent: opts.accent,
        spec: opts.spec,
        quality: opts.quality,
      })
      scene.setView(opts.view ?? 'front')
      return { renderer: scene, kind: '3d' }
    } catch (err) {
      return {
        renderer: svg(),
        kind: 'svg',
        fallbackReason: err instanceof Error ? err.message : String(err),
      }
    }
  }
  return { renderer: svg(), kind: 'svg' }
}
