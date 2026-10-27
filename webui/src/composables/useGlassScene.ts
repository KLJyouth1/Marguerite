/**
 * 背景质感（云朵 / 海面 / 沙丘）的单一事实源。
 *
 * 场景只影响 `GlassBackdrop.vue` 的画法，不改任何语义令牌 —— 所以切场景不会
 * 让对比度守卫失效（文字始终压在 --bg 或玻璃底板上）。
 * 选择持久化在 localStorage，冷启动时恢复。
 */
import { ref } from 'vue'

export const GLASS_SCENES = ['clouds', 'sea', 'dunes'] as const
export type GlassScene = (typeof GLASS_SCENES)[number]

export const GLASS_SCENE_STORAGE_KEY = 'marguerite.glass-scene'
export const GLASS_SCENE_DEFAULT: GlassScene = 'clouds'

function isGlassScene(value: unknown): value is GlassScene {
  return typeof value === 'string' && (GLASS_SCENES as readonly string[]).includes(value)
}

export function readGlassScene(): GlassScene {
  try {
    const raw = localStorage.getItem(GLASS_SCENE_STORAGE_KEY)
    return isGlassScene(raw) ? raw : GLASS_SCENE_DEFAULT
  } catch {
    return GLASS_SCENE_DEFAULT
  }
}

/** 响应式当前场景：`GlassBackdrop` 与参数面板共用同一份状态。 */
export const glassScene = ref<GlassScene>(GLASS_SCENE_DEFAULT)

export function setGlassScene(scene: GlassScene): void {
  glassScene.value = scene
  try {
    localStorage.setItem(GLASS_SCENE_STORAGE_KEY, scene)
  } catch {
    // 隐私模式 / 配额满：只影响"下次记住"，当前视图仍然生效
  }
}

export function initGlassScene(): void {
  glassScene.value = readGlassScene()
}
