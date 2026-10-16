import { computed, readonly, ref } from 'vue'

/** 对话页字号档位。 */
export const CHAT_FONT_SCALES = ['small', 'default', 'large', 'xlarge'] as const
export type ChatFontScale = typeof CHAT_FONT_SCALES[number]

// 版本化 key：将来若档位/语义变化，可安全回退，而不是用新语义解释旧值。
export const CHAT_FONT_SCALE_STORAGE_KEY = 'marguerite.appearance.chatFontScale.v1'

/**
 * 每档对应的对话区根字号（px）。
 *
 * 为什么只改这一个值就能缩放整页：`assets/foundation.css` 里的字号 token
 * （`--fs-xs/sm/md/lg`…）全部是 **rem** 制，rem 相对根节点 `font-size` 计算。
 * 因此在对话区根节点（`.chat`）设 font-size，页内所有字号按比例级联——
 * 不必新增 token、不必逐个组件改，也不会外溢到侧栏/设置页（只作用于 `.chat` 子树）。
 */
export const CHAT_FONT_SCALE_PX: Record<ChatFontScale, number> = {
  small: 14,
  default: 16,
  large: 18,
  xlarge: 20,
}

/** 未知/缺失值一律回落到 `default`（保持设置项引入前的既有观感）。 */
export function parseChatFontScale(value: unknown): ChatFontScale {
  return CHAT_FONT_SCALES.includes(value as ChatFontScale)
    ? value as ChatFontScale
    : 'default'
}

function storage(): Storage | null {
  if (typeof window === 'undefined') return null
  try {
    return window.localStorage
  } catch {
    return null
  }
}

function readStoredScale(): ChatFontScale {
  try {
    return parseChatFontScale(storage()?.getItem(CHAT_FONT_SCALE_STORAGE_KEY))
  } catch {
    return 'default'
  }
}

// 单一响应式偏好：设置页选项与对话页渲染共用同一份，不会各读各的。
const scale = ref<ChatFontScale>(readStoredScale())

function setScale(next: ChatFontScale): void {
  const normalized = parseChatFontScale(next)
  scale.value = normalized
  try {
    storage()?.setItem(CHAT_FONT_SCALE_STORAGE_KEY, normalized)
  } catch {
    // 受限浏览器上下文仍保留本次页面的偏好，只是不持久化。
  }
}

export function useChatFontScale() {
  return {
    scale: readonly(scale),
    /** 可直接绑到 `:style` 的根字号，例如 `:style="{ fontSize: fontPx }"`。 */
    fontPx: computed(() => `${CHAT_FONT_SCALE_PX[scale.value]}px`),
    setScale,
  }
}
