/**
 * 桌宠皮肤系统 v3 — 全皮肤立体模型化。
 *
 * - `cat`：云绒猫（canvas + DEFAULT_PET_SPEC 3D 渲染）
 * - 精灵 4 款（blue/pink/purple/green）：kind='model'，由 petSpec 的 PetModelSpec
 *   数据驱动渲染（尖耳/花型花环/流光尾/发色）—— 已从「静态图片立绘」升级为立体动态。
 * - 预览统一用 emoji + 主题色（不再依赖图片素材）。
 *
 * `kind: 'image'` 类型保留以兼容旧调用（当前列表无 image 皮肤）。
 */

export interface PetSkin {
  id: string
  name: string
  description: string
  /** canvas=默认猫(3D) / model=数据驱动模型(3D) / image=静态立绘(兼容保留) */
  kind: 'canvas' | 'model' | 'image'
  /** image 类型的图片路径（兼容旧调用；当前皮肤列表不使用） */
  src?: string
  /** 主题色（用于预览渐变/光环/气泡点缀） */
  accent: string
  /** 皮肤选择器预览 emoji */
  emoji: string
  /** model 皮肤的 Spec 查询 key（getPetModelSpec；canvas 默认 cat） */
  model?: string
}

export const PET_SKINS: PetSkin[] = [
  {
    id: 'cat',
    name: '茉莉 · 云绒猫',
    description: '白绒毛 · 尖耳大脸 · 呼吸眨眼摆尾',
    kind: 'canvas',
    accent: 'var(--accent)',
    emoji: '😺',
  },
  {
    id: 'blue',
    name: '雏菊精灵',
    description: '蓝白发 · 细长尖耳 · 雏菊花环 · 流光尾',
    kind: 'model',
    model: 'blue',
    accent: '#7eb8da',
    emoji: '🌼',
  },
  {
    id: 'pink',
    name: '樱花精灵',
    description: '粉白发 · 细长尖耳 · 樱花花环 · 流光尾',
    kind: 'model',
    model: 'pink',
    accent: '#f4a8c0',
    emoji: '🌸',
  },
  {
    id: 'purple',
    name: '薰衣草精灵',
    description: '紫白发 · 细长尖耳 · 薰衣草花环 · 流光尾',
    kind: 'model',
    model: 'purple',
    accent: '#b8a0d8',
    emoji: '💜',
  },
  {
    id: 'green',
    name: '四叶草精灵',
    description: '绿白发 · 细长尖耳 · 四叶草花环 · 流光尾',
    kind: 'model',
    model: 'green',
    accent: '#8ec9a0',
    emoji: '🍀',
  },
]

const SKIN_KEY = 'pet.skin'

export function getCurrentSkin(): PetSkin {
  const saved = localStorage.getItem(SKIN_KEY)
  return PET_SKINS.find((s) => s.id === saved) ?? PET_SKINS[0]
}

export function setCurrentSkin(id: string): void {
  if (PET_SKINS.some((s) => s.id === id)) {
    localStorage.setItem(SKIN_KEY, id)
  }
}